use std::collections::HashMap;
use std::path::PathBuf;

use crate::error::{fail, Res};

pub type Sections = HashMap<String, HashMap<String, String>>;

pub const SAMPLE_CONFIG: &str = r###"
# iobs 同步工具配置文件
# 用 --env <名称> 选择下面任意一个环境段（名字可自由增删）
# 请填写自己的 access_key / secret_key（或使用 GUI 的「连接凭据」）

# 请填写自己的服务地址 / bucket / access_key / secret_key
# 下方地址与桶名只是占位示例，必须替换成实际值

[common]
bucket           =
access_key       =
secret_key       =
small_file_limit = 10MB
chunk_size       = 5MB
timeout          = 60
retry            = 3
token_ttl        = 600

# 环境段名字可自由增删；运行时用 --env <名称> 选择，段内配置优先于 [common]
[outer]
base_url = https://iobs.example.com

[inner]
base_url = https://iobs-inner.example.com
"###;

#[derive(Debug, Clone)]
pub struct Config {
    pub env: String,
    pub base_url: String,
    pub bucket: String,
    pub access_key: String,
    pub secret_key: String,
    pub small_file_limit: u64,
    pub chunk_size: u64,
    pub file_ext: Option<String>,
    pub timeout_secs: u64,
    pub retry: u32,
    pub token_ttl: u64,
    pub insecure: bool,
    pub verbose: bool,
    pub progress: bool,
    /// 实际加载到的配置文件路径；None 表示没找到任何 iobs.ini（用的是内置默认值）。
    /// 用于在界面上如实告知「凭据/配置是从哪来的」，避免看起来凭空多出凭据。
    pub config_path: Option<PathBuf>,
    /// 该路径是否由 --config/-c 显式指定
    pub config_explicit: bool,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            env: "outer".to_string(),
            base_url: String::new(),
            bucket: String::new(),
            access_key: String::new(),
            secret_key: String::new(),
            small_file_limit: 10 * 1024 * 1024,
            chunk_size: 5 * 1024 * 1024,
            file_ext: None,
            timeout_secs: 60,
            retry: 3,
            token_ttl: 600,
            insecure: false,
            verbose: false,
            progress: true,
            config_path: None,
            config_explicit: false,
        }
    }
}

/// 解析 INI：`[section]`、`key = value`、`;` 或 `#` 注释
pub fn parse_ini(text: &str) -> Sections {
    let mut sections: Sections = HashMap::new();
    let mut cur = String::from("common");
    sections.insert(cur.clone(), HashMap::new());
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            cur = line[1..line.len() - 1].trim().to_string();
            sections.entry(cur.clone()).or_insert_with(HashMap::new);
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let v = v.trim();
            // 去掉行内注释
            let v = match v.find(|c| c == ';' || c == '#') {
                Some(i) => v[..i].trim(),
                None => v,
            };
            sections
                .entry(cur.clone())
                .or_insert_with(HashMap::new)
                .insert(k.trim().to_lowercase(), v.to_string());
        }
    }
    sections
}

/// 支持 1024/1024KB/10MB/1GB，纯数字按字节
pub fn parse_size(s: &str) -> Option<u64> {
    let t = s.trim().to_uppercase();
    let t = t.replace("IB", "B"); // KiB -> KB
    let num: String = t.chars().take_while(|c| c.is_ascii_digit() || *c == '.').collect();
    let unit: String = t.chars().filter(|c| c.is_alphabetic()).collect();
    let n: f64 = num.parse().ok()?;
    let mul: u64 = match unit.as_str() {
        "" | "B" => 1,
        "K" | "KB" => 1024,
        "M" | "MB" => 1024 * 1024,
        "G" | "GB" => 1024 * 1024 * 1024,
        "T" | "TB" => 1024u64 * 1024 * 1024 * 1024,
        _ => return None,
    };
    Some((n * mul as f64) as u64)
}

/// 配置文件查找顺序：--config > 当前目录 > exe 同目录 > %APPDATA%
pub fn find_config(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        let p = PathBuf::from(p);
        if p.exists() {
            return Some(p);
        }
        return None;
    }
    let mut cands: Vec<PathBuf> = Vec::new();
    cands.push(PathBuf::from("iobs.ini"));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            cands.push(dir.join("iobs.ini"));
        }
    }
    if let Ok(app) = std::env::var("APPDATA") {
        cands.push(PathBuf::from(app).join("sync-iobs").join("iobs.ini"));
    }
    cands.into_iter().find(|p| p.exists())
}

fn env_var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v.trim().to_string()),
        _ => None,
    }
}

/// 读取配置文件（找不到时返回空表）
pub fn load_sections(opts: &[(String, String)]) -> Sections {
    let explicit = crate::cli::opt(opts, "--config").or_else(|| crate::cli::opt(opts, "-c"));
    match find_config(explicit.as_deref()).and_then(|p| std::fs::read_to_string(p).ok()) {
        Some(text) => parse_ini(&text),
        None => Sections::new(),
    }
}

/// 组装最终配置，优先级：命令行 > 环境变量 > 环境段 > common 段 > 内置默认
pub fn resolve(opts: &[(String, String)]) -> Res<Config> {
    resolve_sections(&load_sections(opts), opts)
}

/// 宽松解析：不强制 ak/sk 必填（GUI 启动时用，缺密钥也能开界面）
pub fn resolve_sections_relaxed(sections: &Sections, opts: &[(String, String)]) -> Res<Config> {
    build_config(sections, opts, false)
}

/// 用已解析的配置表组装配置（GUI 切换环境时复用，避免重复读文件）
pub fn resolve_sections(sections: &Sections, opts: &[(String, String)]) -> Res<Config> {
    build_config(sections, opts, true)
}

fn build_config(sections: &Sections, opts: &[(String, String)], strict: bool) -> Res<Config> {
    let mut cfg = Config::default();

    // 记录配置来源（供界面展示「当前配置从哪来」），与 load_sections 用同一套查找顺序
    let explicit = crate::cli::opt(opts, "--config").or_else(|| crate::cli::opt(opts, "-c"));
    cfg.config_explicit = explicit.is_some();
    cfg.config_path = find_config(explicit.as_deref());

    {
        let common = sections.get("common").cloned().unwrap_or_default();

        let env_name = crate::cli::opt(opts, "--env")
            .or_else(|| env_var("IOBS_ENV"))
            .unwrap_or_else(|| "outer".to_string());
        cfg.env = env_name.clone();
        let env_sec = sections.get(&env_name).cloned().unwrap_or_default();

        let pick = |key: &str| -> Option<String> {
            env_sec.get(key).or_else(|| common.get(key)).cloned()
        };

        if let Some(v) = pick("base_url") {
            cfg.base_url = v;
        }
        if let Some(v) = pick("bucket") {
            cfg.bucket = v;
        }
        if let Some(v) = pick("access_key") {
            cfg.access_key = v;
        }
        if let Some(v) = pick("secret_key") {
            cfg.secret_key = v;
        }
        if let Some(v) = pick("small_file_limit").and_then(|s| parse_size(&s)) {
            cfg.small_file_limit = v;
        }
        if let Some(v) = pick("chunk_size").and_then(|s| parse_size(&s)) {
            cfg.chunk_size = v;
        }
        if let Some(v) = pick("file_ext") {
            cfg.file_ext = Some(v.trim_start_matches('.').to_string());
        }
        if let Some(v) = pick("timeout").and_then(|s| s.parse().ok()) {
            cfg.timeout_secs = v;
        }
        if let Some(v) = pick("retry").and_then(|s| s.parse().ok()) {
            cfg.retry = v;
        }
        if let Some(v) = pick("token_ttl").and_then(|s| s.parse().ok()) {
            cfg.token_ttl = v;
        }
        if let Some(v) = pick("insecure") {
            cfg.insecure = v == "1" || v.eq_ignore_ascii_case("true");
        }
    }

    // 环境变量（优先于配置文件）
    if let Some(v) = env_var("IOBS_BASE_URL") {
        cfg.base_url = v;
    }
    if let Some(v) = env_var("IOBS_BUCKET") {
        cfg.bucket = v;
    }
    if let Some(v) = env_var("IOBS_AK") {
        cfg.access_key = v;
    }
    if let Some(v) = env_var("IOBS_SK") {
        cfg.secret_key = v;
    }

    // 命令行最高优先级
    if let Some(v) = crate::cli::opt(opts, "--base-url") {
        cfg.base_url = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--bucket") {
        cfg.bucket = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--ak") {
        cfg.access_key = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--sk") {
        cfg.secret_key = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--timeout").and_then(|s| s.parse().ok()) {
        cfg.timeout_secs = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--retry").and_then(|s| s.parse().ok()) {
        cfg.retry = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--chunk").and_then(|s| parse_size(&s)) {
        cfg.chunk_size = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--ext") {
        cfg.file_ext = Some(v.trim_start_matches('.').to_string());
    }
    if let Some(v) = crate::cli::opt(opts, "--small-limit").and_then(|s| parse_size(&s)) {
        cfg.small_file_limit = v;
    }
    if let Some(v) = crate::cli::opt(opts, "--token-ttl").and_then(|s| s.parse().ok()) {
        cfg.token_ttl = v;
    }
    if crate::cli::has(opts, "--insecure") {
        cfg.insecure = true;
    }
    if crate::cli::has(opts, "--verbose") {
        cfg.verbose = true;
    }
    if crate::cli::has(opts, "--no-progress") {
        cfg.progress = false;
    }

    cfg.base_url = cfg.base_url.trim_end_matches('/').to_string();
    if strict && (cfg.access_key.is_empty() || cfg.secret_key.is_empty()) {
        return fail(2, "缺少 access_key / secret_key：请配置 iobs.ini，或用 --ak/--sk、IOBS_AK/IOBS_SK 提供");
    }
    // 地址与桶名不再内置任何默认值，缺了要明确报出来，避免拼出莫名其妙的 URL
    if strict && cfg.base_url.is_empty() {
        return fail(2, "缺少 base_url：请在 iobs.ini 的环境段里配置，或用 --base-url / IOBS_BASE_URL 提供");
    }
    if strict && cfg.bucket.is_empty() {
        return fail(2, "缺少 bucket：请在 iobs.ini 的 [common] 里配置，或用 --bucket / IOBS_BUCKET 提供");
    }
    if cfg.chunk_size == 0 {
        cfg.chunk_size = 5 * 1024 * 1024;
    }
    Ok(cfg)
}

/// GUI 保存配置时用的落盘路径：与 find_config 同序，找不到则落在 exe 同目录
pub fn default_config_path() -> std::path::PathBuf {
    if let Some(p) = find_config(None) {
        return p;
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            return dir.join("iobs.ini");
        }
    }
    std::path::PathBuf::from("iobs.ini")
}

/// 把内存里的配置段写回 INI 文件（GUI 保存凭据时用）
pub fn write_sections(path: &std::path::Path, sections: &Sections) -> std::io::Result<()> {
    let mut out = String::new();
    for (sec, kv) in sections {
        if sec == "common" && kv.is_empty() {
            continue;
        }
        out.push_str(&format!("[{}]\n", sec));
        for (k, v) in kv {
            out.push_str(&format!("{} = {}\n", k, v));
        }
        out.push('\n');
    }
    std::fs::write(path, out.trim_end().to_string() + "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ini_and_size() {
        let s = parse_ini(SAMPLE_CONFIG);
        assert_eq!(s["outer"]["base_url"], "https://iobs.example.com");
        assert_eq!(s["inner"]["base_url"], "https://iobs-inner.example.com");
        assert_eq!(parse_size("10MB"), Some(10 * 1024 * 1024));
        assert_eq!(parse_size("512"), Some(512));
        assert_eq!(parse_size("1GB"), Some(1024 * 1024 * 1024));
    }
}
