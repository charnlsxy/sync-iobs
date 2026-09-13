mod cli;
mod config;
mod log;
mod error;
#[cfg(feature = "gui")]
mod gui;
mod http;
mod iobs;
mod progress;
mod token;

use std::fs;
use std::path::PathBuf;

use crate::error::{fail, AppError, Res};
use crate::progress::{human_bytes, Progress};

const USAGE: &str = r###"
sync-iobs — iobs 文件双向同步工具

用法:
  sync-iobs upload   <本地文件> [--key <key>] [--name <文件名>] [--ext <后缀>]
                     [--force-small | --force-multipart] [--chunk 5MB]
  sync-iobs download <key> [--out <本地路径>] [--resume]
  sync-iobs url      <key>
  sync-iobs history          列出最近上传的 10 条记录
  sync-iobs init-config [路径]

环境/连接:
  --env <名称>        选择配置文件中的环境段（默认 outer）
  --base-url <url>    直接指定服务地址，优先级最高
  --bucket <bucket>   桶名
  --ak <access_key>   密钥（也可 IOBS_AK 环境变量）
  --sk <secret_key>   密钥（也可 IOBS_SK 环境变量）
  -c, --config <文件> 指定配置文件（默认依次查找 ./iobs.ini、exe 同目录、%APPDATA%）
  --insecure          跳过证书校验（内网自签证书场景）
  --timeout <秒>      单次请求超时（默认 60）
  --retry <次数>      失败重试次数（默认 3）
  --chunk <大小>      分片大小（默认 5MB）
  --small-limit <大小> 小文件阈值，超过则分片（默认 10MB）

其它:
  --no-progress       关闭进度条
  -v, --verbose       打印请求与响应，便于对照调试

key 默认取本地文件名；--key 支持占位符 {file} {name} {ext} {ts} {date}
--ext 只改变服务端保存的文件名后缀，不影响 key
"###;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(argv) {
        eprintln!("错误：{}", e);
        std::process::exit(e.code);
    }
}

fn run(argv: Vec<String>) -> Res<()> {
    let (pos, opts) = cli::parse_argv(argv);
    if pos.is_empty() {
        #[cfg(feature = "gui")]
        {
            let sections = config::load_sections(&opts);
            let cfg = config::resolve_sections_relaxed(&sections, &opts)?;
            return gui::run(cfg, sections).map_err(|e| AppError::new(1, e.to_string()));
        }
        #[cfg(not(feature = "gui"))]
        {
            println!("{}", USAGE);
            return Ok(());
        }
    }
    if cli::has(&opts, "--help") {
        println!("{}", USAGE);
        return Ok(());
    }
    #[cfg(feature = "gui")]
    if pos[0] == "gui" {
        let sections = config::load_sections(&opts);
        let cfg = config::resolve_sections_relaxed(&sections, &opts)?;
        return gui::run(cfg, sections).map_err(|e| AppError::new(1, e.to_string()));
    }

    let cmd = pos[0].as_str();
    let args: Vec<String> = pos[1..].to_vec();

    if cmd == "init-config" {
        let path = PathBuf::from(args.first().cloned().unwrap_or_else(|| "iobs.ini".to_string()));
        if path.exists() {
            return fail(5, format!("{} 已存在，未覆盖", path.display()));
        }
        fs::write(&path, config::SAMPLE_CONFIG.trim_start())?;
        println!("已生成配置文件：{}", path.display());
        return Ok(());
    }

    let cfg = config::resolve(&opts)?;
    let small_limit = cfg.small_file_limit;
    let chunk = cfg.chunk_size;
    let show_progress = cfg.progress;
    let client = http::Client::new(&cfg, None)?;
    let io = iobs::Iobs::new(cfg, client);

    match cmd {
        "upload" => cmd_upload(&io, &args, &opts, small_limit, chunk, show_progress),
        "download" => cmd_download(&io, &args, &opts, show_progress),
        "url" => {
            let key = args.first().ok_or_else(|| AppError::new(2, "用法: sync-iobs url <key>"))?;
            println!("{}", io.download_url(key));
            Ok(())
        }
        "history" => {
            let items = io.load_history();
            if items.is_empty() {
                println!("（暂无上传历史）");
            } else {
                println!("时间(Unix)\t原始文件名\tkey");
                for it in &items {
                    println!("{}\t{}\t{}", it.time, it.name, it.key);
                }
            }
            Ok(())
        }
        _ => fail(2, format!("未知命令：{}（用 --help 查看用法）", cmd)),
    }
}

fn cmd_upload(
    io: &iobs::Iobs,
    args: &[String],
    opts: &[(String, String)],
    small_limit: u64,
    chunk: u64,
    show_progress: bool,
) -> Res<()> {
    let file = args
        .first()
        .ok_or_else(|| AppError::new(2, "用法: sync-iobs upload <本地文件>"))?;
    let path = PathBuf::from(file);
    if !path.exists() {
        return fail(5, format!("文件不存在：{}", file));
    }
    let size = fs::metadata(&path)?.len();
    let key = resolve_key(opts, &path);
    let name = resolve_name(opts, &path, io.cfg.file_ext.as_deref());

    let use_small = if cli::has(opts, "--force-small") {
        true
    } else if cli::has(opts, "--force-multipart") {
        false
    } else {
        size < small_limit
    };

    println!("文件：{}（{}）", path.display(), human_bytes(size));
    println!("key ：{}", key);
    println!("名称：{}", name);
    println!(
        "方式：{}",
        if use_small {
            format!("小文件直传")
        } else {
            format!("分片上传（每片 {}）", human_bytes(chunk.max(iobs::MIN_CHUNK)))
        }
    );

    let mut prog = Progress::new("上传", size, show_progress);
    let url = if use_small {
        io.upload_small(&path, &key, &name, &mut prog)?
    } else {
        io.upload_multipart(&path, &key, &name, &mut prog)?
    };
    prog.finish();
    println!("上传完成");
    println!("下载地址：{}", url);
    // 记录到最近上传历史（失败不阻断主流程）
    if let Err(e) = io.record_upload(&key, &name) {
        eprintln!("（历史记录更新失败：{}）", e);
    }
    Ok(())
}

fn cmd_download(
    io: &iobs::Iobs,
    args: &[String],
    opts: &[(String, String)],
    show_progress: bool,
) -> Res<()> {
    let key = args
        .first()
        .ok_or_else(|| AppError::new(2, "用法: sync-iobs download <key>"))?;
    let out: PathBuf = match cli::opt(opts, "--out") {
        Some(o) => {
            let p = PathBuf::from(&o);
            if o.ends_with('/') || o.ends_with('\\') || p.is_dir() {
                p.join(safe_name(key))
            } else {
                p
            }
        }
        None => PathBuf::from(safe_name(key)),
    };
    let resume = cli::has(opts, "--resume");

    println!("下载：{} -> {}", key, out.display());
    let mut prog = Progress::new("下载", 0, show_progress);
    let n = io.download(key, &out, resume, &mut prog)?;
    prog.finish();
    println!("下载完成：{}（{}）", out.display(), human_bytes(n));
    Ok(())
}

fn resolve_key(opts: &[(String, String)], path: &std::path::Path) -> String {
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| file_name.clone());
    let ext = path
        .extension()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    match cli::opt(opts, "--key") {
        Some(k) => k
            .replace("{file}", &file_name)
            .replace("{name}", &stem)
            .replace("{ext}", &ext)
            .replace("{ts}", &token::now_secs().to_string())
            .replace("{date}", &today()),
        None => file_name,
    }
}

fn resolve_name(opts: &[(String, String)], path: &std::path::Path, default_ext: Option<&str>) -> String {
    let file_name = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "file".to_string());
    if let Some(n) = cli::opt(opts, "--name") {
        return n;
    }
    match cli::opt(opts, "--ext").or_else(|| default_ext.map(|s| s.to_string())) {
        Some(ext) => {
            let ext = ext.trim_start_matches('.');
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file_name.clone());
            if ext.is_empty() {
                stem
            } else {
                format!("{}.{}", stem, ext)
            }
        }
        None => file_name,
    }
}

fn safe_name(key: &str) -> String {
    key.replace('/', "_").replace('\\', "_")
}

/// 从 unix 秒算出 YYYYMMDD（Howard Hinnant 的 civil_from_days）
fn today() -> String {
    let days = (token::now_secs() / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    format!("{:04}{:02}{:02}", y, m, d)
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as i64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}
