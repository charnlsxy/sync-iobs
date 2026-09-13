use std::io::{Read, Write};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::error::{AppError, Res};
use crate::log::LogBuf;
use crate::progress::Progress;

pub struct Client {
    agent: ureq::Agent,
    retry: u32,
    pub verbose: bool,
    /// GUI 日志窗口用的共享缓冲；CLI 传 None（只走 stderr）
    log: Option<LogBuf>,
}

impl Client {
    pub fn new(cfg: &Config, log: Option<LogBuf>) -> Res<Client> {
        let t = Duration::from_secs(cfg.timeout_secs.max(1));
        let mut b = ureq::AgentBuilder::new()
            .timeout_connect(t)
            .timeout_read(t)
            .timeout_write(t)
            .redirects(3);
        // ureq 2.x 的 native-tls 只是适配器，必须显式挂到 Agent 上（否则 https 无后端）
        let conn = if cfg.insecure {
            native_tls::TlsConnector::builder()
                .danger_accept_invalid_certs(true)
                .danger_accept_invalid_hostnames(true)
                .build()?
        } else {
            native_tls::TlsConnector::new()?
        };
        b = b.tls_connector(Arc::new(conn));
        Ok(Client {
            agent: b.build(),
            retry: cfg.retry,
            verbose: cfg.verbose,
            log,
        })
    }

    /// 同时写 stderr（CLI 可见）与 GUI 日志缓冲
    pub(crate) fn note(&self, level: &'static str, msg: String) {
        eprintln!("{}", msg);
        if let Some(log) = &self.log {
            crate::log::push(log, level, msg);
        }
    }

    fn sleep_backoff(attempt: u32) {
        let ms = 300u64 * (1u64 << attempt.min(5));
        std::thread::sleep(Duration::from_millis(ms));
    }

    /// POST 字节体，返回响应文本；失败按 retry 次数重试
    pub fn post_bytes(
        &self,
        url: &str,
        body: &[u8],
        content_type: &str,
        headers: &[(&str, &str)],
    ) -> Res<String> {
        let mut attempt = 0u32;
        loop {
            if self.verbose {
                println!("> POST {}", url);
                println!("> Content-Type: {}", content_type);
                println!("> body: {} bytes", body.len());
            }
            let mut req = self.agent.post(url).set("Content-Type", content_type);
            for (k, v) in headers {
                req = req.set(k, v);
            }
            match req.send(body) {
                Ok(resp) => {
                    let s = resp.into_string().unwrap_or_default();
                    if self.verbose {
                        println!("< {}", s);
                    }
                    return Ok(s);
                }
                Err(ureq::Error::Status(code, resp)) => {
                    // 4xx 通常是参数/签名问题，重试没有意义，直接把响应体带出来
                    let text = resp.into_string().unwrap_or_default();
                    let retryable = (code >= 500 && code < 600) || code == 429;
                    if !retryable || attempt >= self.retry {
                        return Err(AppError::new(4, format!("HTTP {} {}", code, text)));
                    }
                    self.note("WARN", format!("  请求失败（第 {} 次重试）：HTTP {} {}", attempt + 1, code, text));
                    Self::sleep_backoff(attempt);
                    attempt += 1;
                }
                Err(e) => {
                    if attempt >= self.retry {
                        return Err(e.into());
                    }
                    self.note("WARN", format!("  请求失败（第 {} 次重试）：{}", attempt + 1, e));
                    Self::sleep_backoff(attempt);
                    attempt += 1;
                }
            }
        }
    }

    /// GET 下载到 writer；start > 0 时带 Range 续传，并自动推断总大小
    pub fn get_into(
        &self,
        url: &str,
        out: &mut dyn Write,
        start: u64,
        prog: &mut Progress,
    ) -> Res<u64> {
        let mut attempt = 0u32;
        loop {
            if self.verbose {
                println!("> GET {}", url);
                if start > 0 {
                    println!("> Range: bytes={}-", start);
                }
            }
            let req = if start > 0 {
                self.agent.get(url).set("Range", &format!("bytes={}-", start))
            } else {
                self.agent.get(url)
            };
            match req.call() {
                Ok(resp) => {
                    let code = resp.status();
                    if code != 200 && code != 206 {
                        return Err(AppError::new(
                            4,
                            format!("下载失败：HTTP {} {}", code, resp.status_text()),
                        ));
                    }
                    let len: u64 = resp
                        .header("Content-Length")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(0);
                    let total = if code == 206 {
                        resp.header("Content-Range")
                            .and_then(|cr| cr.rsplit('/').next())
                            .and_then(|t| t.trim().parse().ok())
                            .unwrap_or(start + len)
                    } else {
                        start + len
                    };
                    prog.set_total(total);

                    let mut reader = resp.into_reader();
                    let mut buf = vec![0u8; 64 * 1024];
                    let mut got = start;
                    loop {
                        let n = reader.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        out.write_all(&buf[..n])?;
                        got += n as u64;
                        prog.set(got);
                    }
                    out.flush()?;
                    return Ok(got);
                }
                Err(e) => {
                    match e {
                        // 4xx 通常是参数/签名/资源不存在等问题，重试没有意义，直接带出响应体
                        ureq::Error::Status(code, resp) => {
                            let text = resp.into_string().unwrap_or_default();
                            let retryable = (code >= 500 && code < 600) || code == 429;
                            if !retryable || attempt >= self.retry {
                                return Err(AppError::new(4, format!("下载失败：HTTP {} {}", code, text)));
                            }
                            self.note("WARN", format!("  下载中断（第 {} 次重试）：HTTP {} {}", attempt + 1, code, text));
                            Self::sleep_backoff(attempt);
                        }
                        other => {
                            if attempt >= self.retry {
                                return Err(other.into());
                            }
                            self.note("WARN", format!("  下载中断（第 {} 次重试）：{}", attempt + 1, other));
                            Self::sleep_backoff(attempt);
                        }
                    }
                    attempt += 1;
                }
            }
        }
    }
}
