use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

/// 一条操作日志：时间戳（Unix 秒）+ 级别 + 文本。
/// 级别约定：INFO / OK / WARN / ERR
#[derive(Clone)]
pub struct LogEntry {
    pub t: u64,
    pub level: &'static str,
    pub msg: String,
}

/// GUI 日志窗口用的共享缓冲（后台线程写、UI 每帧读）
pub type LogBuf = Arc<Mutex<Vec<LogEntry>>>;

pub fn new_buf() -> LogBuf {
    Arc::new(Mutex::new(Vec::new()))
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 追加一条日志（自动截断到最近 2000 条，避免无限增长）
pub fn push(log: &LogBuf, level: &'static str, msg: String) {
    if let Ok(mut v) = log.lock() {
        const CAP: usize = 2000;
        if v.len() >= CAP {
            let drop = v.len() - CAP + 1;
            v.drain(0..drop);
        }
        v.push(LogEntry { t: now_secs(), level, msg });
    }
}

/// 清空日志（GUI「清空日志」按钮）
pub fn clear(log: &LogBuf) {
    if let Ok(mut v) = log.lock() {
        v.clear();
    }
}

/// 取最近 N 条日志的副本（用于 UI 渲染，避免持有锁）
pub fn recent(log: &LogBuf, n: usize) -> Vec<LogEntry> {
    if let Ok(v) = log.lock() {
        let len = v.len();
        let start = len.saturating_sub(n);
        v[start..].to_vec()
    } else {
        Vec::new()
    }
}
