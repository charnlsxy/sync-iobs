use std::io::{self, IsTerminal, Write};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub fn human_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0usize;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{}B", n)
    } else if v < 10.0 {
        format!("{:.2}{}", v, U[i])
    } else if v < 100.0 {
        format!("{:.1}{}", v, U[i])
    } else {
        format!("{:.0}{}", v, U[i])
    }
}

/// UI 模式下共享给 HTTP 服务的任务状态
#[derive(Clone)]
pub struct TaskState {
    pub id: u64,
    pub kind: String,
    pub name: String,
    pub key: String,
    pub out: String,
    pub total: u64,
    pub done: u64,
    pub speed: u64,
    pub status: String, // running / done / error
    pub error: String,
    pub url: String,
}

pub type Shared = Arc<Mutex<Vec<TaskState>>>;

pub struct Progress {
    label: String,
    total: u64,
    done: u64,
    start: Instant,
    last_draw: Instant,
    last_pct: u64,
    tty: bool,
    enabled: bool,
    finished: bool,
    shared: Option<(Shared, u64)>,
}

impl Progress {
    pub fn new(label: &str, total: u64, enabled: bool) -> Progress {
        Progress {
            label: label.to_string(),
            total,
            done: 0,
            start: Instant::now(),
            last_draw: Instant::now(),
            last_pct: 0,
            tty: io::stdout().is_terminal(),
            enabled,
            finished: false,
            shared: None,
        }
    }

    /// UI 模式：不在终端打印，改为更新共享任务状态
    pub fn for_shared(id: u64, shared: Shared) -> Progress {
        Progress {
            label: String::new(),
            total: 0,
            done: 0,
            start: Instant::now(),
            last_draw: Instant::now(),
            last_pct: 0,
            tty: false,
            enabled: false,
            finished: false,
            shared: Some((shared, id)),
        }
    }

    pub fn set_total(&mut self, total: u64) {
        self.total = total;
        self.sync(None);
    }

    pub fn set(&mut self, done: u64) {
        self.done = done;
        self.draw(false);
        self.sync(None);
    }

    pub fn inc(&mut self, n: u64) {
        self.done += n;
        self.draw(false);
        self.sync(None);
    }

    pub fn finish(&mut self) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.draw(true);
        self.sync(Some("done"));
        if self.enabled {
            println!();
        }
    }

    fn sync(&self, status: Option<&str>) {
        if let Some((shared, id)) = &self.shared {
            let speed = self.done as f64 / self.start.elapsed().as_secs_f64().max(0.001);
            if let Ok(mut v) = shared.lock() {
                if let Some(t) = v.iter_mut().find(|t| t.id == *id) {
                    t.done = self.done;
                    t.total = self.total;
                    t.speed = speed as u64;
                    if let Some(s) = status {
                        t.status = s.to_string();
                    }
                }
            }
        }
    }

    fn draw(&mut self, force: bool) {
        if !self.enabled {
            return;
        }
        let now = Instant::now();
        if !force && self.tty && now.duration_since(self.last_draw) < Duration::from_millis(80) {
            return;
        }
        let pct = if self.total > 0 {
            (self.done as f64 / self.total as f64 * 100.0) as u64
        } else {
            0
        };
        if !force && !self.tty && pct / 5 == self.last_pct / 5 && self.done != self.total {
            return;
        }
        self.last_pct = pct;
        self.last_draw = now;

        let line = self.render(pct);
        if self.tty {
            print!("\r{}", line);
            let _ = io::stdout().flush();
        } else {
            println!("{}", line);
        }
    }

    fn render(&self, pct: u64) -> String {
        const W: usize = 26;
        let filled = if self.total > 0 {
            (self.done as f64 / self.total as f64 * W as f64) as usize
        } else {
            0
        };
        let filled = filled.min(W);
        let bar: String = "#".repeat(filled) + &"-".repeat(W - filled);

        let el = self.start.elapsed().as_secs_f64();
        let speed = if el > 0.0 { self.done as f64 / el } else { 0.0 };
        let speed_s = format!("{}/s", human_bytes(speed as u64));
        let eta_s = if self.total > self.done && speed > 1.0 {
            let s = ((self.total - self.done) as f64 / speed) as u64;
            format!("  eta {}s", s)
        } else {
            String::new()
        };
        let size = if self.total > 0 {
            format!("  {}/{}", human_bytes(self.done), human_bytes(self.total))
        } else {
            format!("  {}", human_bytes(self.done))
        };
        format!(
            "{} [{}] {:>3}%{}  {}{}",
            self.label, bar, pct, size, speed_s, eta_s
        )
    }
}
