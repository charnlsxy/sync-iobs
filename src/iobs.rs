use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use crate::config::Config;
use crate::error::{fail, Res};
use crate::http::Client;
use crate::progress::{human_bytes, Progress};
use crate::token::Signer;

use serde::{Deserialize, Serialize};

const UA: &str = "csp-iobs-java-sdk/1.3.4";

/// 服务端对分片大小的下限（实测，除最后一片外每片需 >= 5MB）
pub const MIN_CHUNK: u64 = 5 * 1024 * 1024;

/// 最近上传历史的固定 iobs key（这个 key 自身也是一份 JSON 文件）
pub const HISTORY_KEY: &str = "cztesthistoryupload10";

/// 单条历史记录：上传 key + 原始文件名 + 上传时间（Unix 秒）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryItem {
    pub key: String,
    pub name: String,
    pub time: u64,
}

#[derive(Serialize, Deserialize)]
struct HistoryFile {
    items: Vec<HistoryItem>,
}

pub fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for &b in s.as_bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// 极简 JSON 取值：够用即可（响应只含 bucket/hash/key/uploadId 这类扁平字段）
pub fn json_get(s: &str, key: &str) -> Option<String> {
    let pat = format!("\"{}\"", key);
    let i = s.find(&pat)?;
    let rest = s[i + pat.len()..].trim_start().strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub struct Iobs {
    pub cfg: Config,
    pub client: Client,
    signer: Signer,
}

impl Iobs {
    pub fn new(cfg: Config, client: Client) -> Iobs {
        let signer = Signer::new(&cfg.access_key, &cfg.secret_key);
        Iobs { cfg, client, signer }
    }

    pub fn token(&self, key: &str) -> String {
        self.signer.token(&self.cfg.bucket, key, self.cfg.token_ttl)
    }

    pub fn download_url(&self, key: &str) -> String {
        format!(
            "{}/download/{}/{}?token={}",
            self.cfg.base_url,
            percent_encode(&self.cfg.bucket),
            percent_encode(key),
            self.token(key)
        )
    }

    /// 小文件直传：multipart/form-data，一次请求完成
    pub fn upload_small(&self, path: &Path, key: &str, name: &str, prog: &mut Progress) -> Res<String> {
        let data = fs::read(path)?;
        self.upload_small_bytes(key, name, &data, prog)
    }

    /// 小文件直传（内存字节版本）：供普通上传与历史文件写回复用
    pub fn upload_small_bytes(&self, key: &str, name: &str, data: &[u8], prog: &mut Progress) -> Res<String> {
        let size = data.len() as u64;
        prog.set_total(size);
        if size > 0 {
            let boundary = format!("---------------------------{}", millis());
            let token = self.token(key);
            self.client.note("INFO", format!("小文件直传：key={} 文件名={} 大小={}", key, name, human_bytes(size)));

            let mut body: Vec<u8> = Vec::with_capacity(size as usize + 1024);
            body.extend_from_slice(
                format!(
                    "--{}\r\nContent-Disposition: form-data; name=\"token\"\r\n\r\n{}\r\n",
                    boundary, token
                )
                .as_bytes(),
            );
            body.extend_from_slice(
                format!(
                    "--{}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{}\"\r\nContent-Type: multipart/form-data; charset=ISO-8859-1\r\n\r\n",
                    boundary, name
                )
                .as_bytes(),
            );

            let mut off = 0usize;
            let mut buf = vec![0u8; 256 * 1024];
            while off < data.len() {
                let want = (buf.len() as usize).min(data.len() - off);
                buf[..want].copy_from_slice(&data[off..off + want]);
                body.extend_from_slice(&buf[..want]);
                off += want;
                prog.inc(want as u64);
            }
            body.extend_from_slice(format!("\r\n--{}--\r\n", boundary).as_bytes());

            let url = format!(
                "{}/upload/{}/{}",
                self.cfg.base_url,
                percent_encode(&self.cfg.bucket),
                percent_encode(key)
            );
            let ct = format!("multipart/form-data; boundary={}", boundary);
            self.client
                .post_bytes(&url, &body, &ct, &[("User-Agent", UA)])?;
            self.client.note("OK", format!("上传成功：{}", key));
        }
        Ok(self.download_url(key))
    }

    /// 大文件分片：initUploadPart -> uploadPart x N -> completeUpload
    pub fn upload_multipart(&self, path: &Path, key: &str, name: &str, prog: &mut Progress) -> Res<String> {
        let size = fs::metadata(path)?.len();
        prog.set_total(size);
        // 实测：服务端要求除最后一片外每片 >= 5MB，否则返回 400 EntityTooSmall
        let mut chunk = self.cfg.chunk_size.max(1);
        if chunk < MIN_CHUNK {
            self.client.note(
                "WARN",
                format!(
                    "警告：分片大小 {} 小于服务端下限，已自动调整为 {}",
                    human_bytes(chunk),
                    human_bytes(MIN_CHUNK)
                ),
            );
            chunk = MIN_CHUNK;
        }
        let parts_total = if size == 0 { 1 } else { (size + chunk - 1) / chunk };
        self.client.note(
            "INFO",
            format!(
                "分片上传：key={} 文件名={} 大小={} 分片={} x {}",
                key,
                name,
                human_bytes(size),
                parts_total,
                human_bytes(chunk)
            ),
        );

        let mut token = self.token(key);
        let mut token_at = Instant::now();

        // 1) 初始化，取 uploadId（响应为纯文本；也兼容 JSON 形态）
        let init_url = format!(
            "{}/initUploadPart/{}/{}?token={}&fileSize={}&fileName={}",
            self.cfg.base_url,
            percent_encode(&self.cfg.bucket),
            percent_encode(key),
            token,
            size,
            percent_encode(name)
        );
        let raw = self.client.post_bytes(&init_url, b"", "application/x-www-form-urlencoded", &[("User-Agent", UA)])?;
        let upload_id = if raw.trim_start().starts_with('{') {
            json_get(&raw, "uploadId").or_else(|| json_get(&raw, "data")).unwrap_or_else(|| raw.trim().to_string())
        } else {
            raw.trim().to_string()
        };
        if upload_id.is_empty() {
            return fail(4, format!("初始化分片上传失败，响应：{}", raw));
        }
        if self.cfg.verbose {
            println!("uploadId = {}", upload_id);
        }

        // 2) 逐片上传
        let mut f = File::open(path)?;
        let mut hashes: Vec<String> = Vec::with_capacity(parts_total as usize);
        let mut sent: u64 = 0;
        for idx in 0..parts_total {
            let n = chunk.min(size - sent);
            let mut buf = vec![0u8; n as usize];
            f.read_exact(&mut buf)?;

            // token 有效期默认是 600 秒，长传需要续签
            if token_at.elapsed().as_secs() + 60 >= self.cfg.token_ttl {
                token = self.token(key);
                token_at = Instant::now();
                if self.cfg.verbose {
                    println!("token 已续签");
                }
            }

            let url = format!(
                "{}/uploadPart/{}/{}?token={}&uploadId={}&partNumber={}",
                self.cfg.base_url,
                percent_encode(&self.cfg.bucket),
                percent_encode(key),
                token,
                percent_encode(&upload_id),
                idx + 1
            );
            let resp = self
                .client
                .post_bytes(&url, &buf, "application/octet-stream", &[("User-Agent", UA)])?;
            let hash = json_get(&resp, "hash").unwrap_or_else(|| resp.trim().to_string());
            if hash.is_empty() {
                return fail(4, format!("第 {} 片上传未返回 hash，响应：{}", idx + 1, resp));
            }
            hashes.push(hash);
            sent += n;
            prog.set(sent);
            if self.cfg.verbose {
                println!("  分片 {}/{} 完成", idx + 1, parts_total);
            }
        }

        // 3) 合并
        let mut map = String::from("{");
        for (i, h) in hashes.iter().enumerate() {
            if i > 0 {
                map.push(',');
            }
            map.push_str(&format!("\"{}\":\"{}\"", i + 1, h));
        }
        map.push('}');
        let body = format!("partMap={}", percent_encode(&map));
        let done_url = format!(
            "{}/completeUpload/{}/{}?token={}&uploadId={}",
            self.cfg.base_url,
            percent_encode(&self.cfg.bucket),
            percent_encode(key),
            self.token(key),
            percent_encode(&upload_id)
        );
        self.client
            .post_bytes(&done_url, body.as_bytes(), "application/x-www-form-urlencoded; charset=utf-8", &[("User-Agent", UA)])?;

        self.client.note("OK", format!("上传成功（分片合并）：{}", key));
        Ok(self.download_url(key))
    }

    /// 下载到本地文件；resume=true 时使用 .part 临时文件 + Range 续传
    pub fn download(&self, key: &str, out: &Path, resume: bool, prog: &mut Progress) -> Res<u64> {
        let url = self.download_url(key);
        self.client.note("INFO", format!("开始下载：{} -> {}", key, out.display()));
        if let Some(parent) = out.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                fs::create_dir_all(parent)?;
            }
        }

        let part: Option<PathBuf> = if resume {
            Some(PathBuf::from(format!("{}.part", out.display())))
        } else {
            None
        };
        let target: PathBuf = match &part {
            Some(p) => p.clone(),
            None => out.to_path_buf(),
        };
        let start = if resume && target.exists() {
            fs::metadata(&target)?.len()
        } else {
            0
        };

        let mut file: Box<dyn Write> = if start > 0 {
            Box::new(OpenOptions::new().append(true).open(&target)?)
        } else {
            Box::new(File::create(&target)?)
        };

        let got = match self.client.get_into(&url, &mut *file, start, prog) {
            Ok(v) => v,
            Err(e) => {
                if part.is_some() {
                    self.client.note("WARN", format!("下载中断，已保留 {}，重新执行 --resume 可续传", target.display()));
                }
                return Err(e);
            }
        };
        drop(file);

        if let Some(p) = &part {
            fs::rename(p, out)?;
        }
        self.client.note(
            "OK",
            format!("下载完成：{} -> {}（{}）", key, out.display(), human_bytes(got)),
        );
        Ok(got)
    }

    /// 读取最近上传历史：下载 HISTORY_KEY 并解析；key 不存在或解析失败都返回空
    pub fn load_history(&self) -> Vec<HistoryItem> {
        let url = self.download_url(HISTORY_KEY);
        let mut buf: Vec<u8> = Vec::new();
        let mut prog = Progress::new("history", 0, false);
        match self.client.get_into(&url, &mut buf, 0, &mut prog) {
            Ok(_) => parse_history(&buf).unwrap_or_default(),
            Err(_) => Vec::new(),
        }
    }

    /// 记录一次上传到最近历史：读旧记录 → 去重并置顶 → 截断 10 条 → 写回 iobs
    pub fn record_upload(&self, key: &str, name: &str) -> Res<()> {
        let mut items = self.load_history();
        let now = crate::token::now_secs();
        items.retain(|it| it.key != key);
        items.insert(0, HistoryItem { key: key.to_string(), name: name.to_string(), time: now });
        items.truncate(10);
        let json = serialize_history(&items);
        // 历史文件以 .png 作为服务端文件名后缀（bucket 会拒绝部分后缀，.png 实测可用）
        let fname = format!("{}.png", HISTORY_KEY);
        let mut prog = Progress::new("history", 0, false);
        self.upload_small_bytes(HISTORY_KEY, &fname, json.as_bytes(), &mut prog)?;
        self.client.note("OK", format!("最近上传已更新：{} 条记录", items.len()));
        Ok(())
    }
}

fn serialize_history(items: &[HistoryItem]) -> String {
    let f = HistoryFile { items: items.to_vec() };
    serde_json::to_string(&f).unwrap_or_else(|_| "{\"items\":[]}".to_string())
}

fn parse_history(buf: &[u8]) -> Option<Vec<HistoryItem>> {
    let s = std::str::from_utf8(buf).ok()?;
    let f: HistoryFile = serde_json::from_str(s).ok()?;
    Some(f.items)
}

fn millis() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_and_encode() {
        let s = r#"{"bucket":"b","hash":"abc123","key":"k"}"#;
        assert_eq!(json_get(s, "hash"), Some("abc123".to_string()));
        assert_eq!(json_get(s, "none"), None);
        assert_eq!(percent_encode("a b.txt"), "a%20b.txt");
    }
}
