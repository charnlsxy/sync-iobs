use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eframe::egui;

use crate::config::{self, Config, Sections};
use crate::error::Res;
use crate::http::Client;
use crate::iobs::{HistoryItem, Iobs, HISTORY_KEY};
use crate::log::{self, LogBuf};
use crate::progress::{human_bytes, Progress, Shared, TaskState};

// —— 设计色板（浅色）——
const ACCENT: egui::Color32 = egui::Color32::from_rgb(37, 99, 235);
const ACCENT_SOFT: egui::Color32 = egui::Color32::from_rgb(239, 246, 255);
const PAGE_BG: egui::Color32 = egui::Color32::from_rgb(247, 248, 250);
const CARD_BG: egui::Color32 = egui::Color32::from_rgb(255, 255, 255);
const BORDER: egui::Color32 = egui::Color32::from_rgb(229, 231, 235);
const BORDER_STRONG: egui::Color32 = egui::Color32::from_rgb(209, 213, 219);
const TEXT: egui::Color32 = egui::Color32::from_rgb(17, 24, 39);
const TEXT_MUTED: egui::Color32 = egui::Color32::from_rgb(107, 114, 128);
const TEXT_FAINT: egui::Color32 = egui::Color32::from_rgb(156, 163, 175);
const OK: egui::Color32 = egui::Color32::from_rgb(22, 163, 74);
const OK_SOFT: egui::Color32 = egui::Color32::from_rgb(236, 253, 245);
const WARN: egui::Color32 = egui::Color32::from_rgb(217, 119, 6);
const DANGER: egui::Color32 = egui::Color32::from_rgb(220, 38, 38);
const DANGER_SOFT: egui::Color32 = egui::Color32::from_rgb(254, 242, 242);

pub fn run(cfg: Config, sections: Sections) -> Result<(), Box<dyn std::error::Error>> {
    hide_console();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([880.0, 660.0])
            .with_min_inner_size([720.0, 520.0])
            .with_title("sync-iobs — iobs 文件同步"),
        ..Default::default()
    };
    eframe::run_native(
        "sync-iobs",
        options,
        Box::new(move |cc| {
            setup_style(&cc.egui_ctx);
            let font_used = setup_fonts(&cc.egui_ctx);
            Ok(Box::new(App::new(cfg, sections, font_used)))
        }),
    )?;
    Ok(())
}

fn setup_style(ctx: &egui::Context) {
    let mut style = (*ctx.style_of(egui::Theme::Light)).clone();
    style.visuals = egui::Visuals::light();

    // 留白：比默认松一些，缓解「太拥挤」
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(16.0, 7.0);

    let v = &mut style.visuals;
    v.panel_fill = PAGE_BG;
    v.window_fill = PAGE_BG;
    v.extreme_bg_color = CARD_BG;
    v.selection.bg_fill = ACCENT_SOFT;
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);

    let r = egui::CornerRadius::same(6);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }

    // 输入类控件：白底 + 细边框，聚焦时强调色描边
    v.widgets.inactive.bg_fill = CARD_BG;
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_fill = CARD_BG;
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, BORDER_STRONG);
    v.widgets.active.bg_fill = CARD_BG;
    v.widgets.active.bg_stroke = egui::Stroke::new(1.5, ACCENT);
    v.widgets.open.bg_fill = CARD_BG;
    v.widgets.open.bg_stroke = egui::Stroke::new(1.5, ACCENT);

    ctx.set_style_of(egui::Theme::Light, style);
    ctx.set_theme(egui::Theme::Light);
}

/// 卡片容器：白底 + 细边框 + 圆角 + 内边距（替掉原来厚重的 group 框）
fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::NONE
        .fill(CARD_BG)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add(ui)
        })
        .inner
}

/// 分区标题
fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(13.5).color(TEXT).strong());
}

/// 表单行：标签固定宽度且右对齐，控件起点统一（解决排版参差）
fn form_row<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(
            egui::vec2(54.0, 22.0),
            egui::Layout::right_to_left(egui::Align::Center),
            |ui| {
                ui.label(egui::RichText::new(label).color(TEXT_MUTED).small());
            },
        );
        add(ui)
    })
    .inner
}

/// 主操作按钮
fn primary_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(egui::Color32::WHITE).strong())
            .fill(ACCENT)
            .corner_radius(egui::CornerRadius::same(6))
            .min_size(egui::vec2(88.0, 28.0)),
    )
    .clicked()
}

/// 次要按钮
fn ghost_button(ui: &mut egui::Ui, text: &str) -> bool {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(TEXT_MUTED))
            .fill(CARD_BG)
            .stroke(egui::Stroke::new(1.0, BORDER))
            .corner_radius(egui::CornerRadius::same(6))
            .min_size(egui::vec2(72.0, 28.0)),
    )
    .clicked()
}

/// 状态小标签
fn chip(ui: &mut egui::Ui, text: &str, fg: egui::Color32, bg: egui::Color32) {
    egui::Frame::NONE
        .fill(bg)
        .corner_radius(egui::CornerRadius::same(4))
        .inner_margin(egui::Margin::same(4))
        .show(ui, |ui| {
            ui.label(egui::RichText::new(text).color(fg).small().strong());
        });
}

/// egui 内置字体不含中文，必须挂一个系统中文字体，否则中文全是方块。
///
/// 按平台依次尝试：Windows → macOS → Linux。
/// 注意 macOS 的中文字体多是 `.ttc`（字体集合），egui 0.36 底层用
/// `skrifa::FontRef::from_index` 解析，原生支持 ttc，取 index 0（PingFang SC 字面）即可。
///
/// 返回实际加载成功的字体路径；None 表示都没找到（界面中文会显示异常）。
fn setup_fonts(ctx: &egui::Context) -> Option<String> {
    let candidates: &[&str] = &[
        // Windows
        "C:\\Windows\\Fonts\\simhei.ttf",
        "C:\\Windows\\Fonts\\Deng.ttf",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\simsun.ttc",
        // macOS
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/STHeiti Medium.ttc",
        "/System/Library/Fonts/STHeiti Light.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/Supplemental/Songti.ttc",
        "/System/Library/Fonts/Supplemental/STHeiti Medium.ttc",
        "/Library/Fonts/Arial Unicode.ttf",
        // Linux
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/usr/share/fonts/truetype/arphic/uming.ttc",
    ];
    let mut fonts = egui::FontDefinitions::default();
    for (i, path) in candidates.iter().enumerate() {
        // 目录项（比如 /System/Library/Fonts/Supplemental/ 不存在时）读出来是空的
        if let Ok(bytes) = std::fs::read(path) {
            if bytes.is_empty() {
                continue;
            }
            let name = format!("cn{}", i);
            fonts
                .font_data
                .insert(name.clone(), egui::FontData::from_owned(bytes).into());
            for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
                fonts.families.entry(family).or_default().insert(0, name.clone());
            }
            ctx.set_fonts(fonts);
            return Some((*path).to_string());
        }
    }
    ctx.set_fonts(fonts);
    None
}

struct App {
    cfg: Config,
    sections: Sections,
    envs: Vec<String>,
    env: String,
    tasks: Shared,
    next_id: u64,

    logs: LogBuf,
    log_auto_scroll: bool,
    /// 操作日志是否展开（默认折叠，避免占地方）
    log_open: bool,
    /// 日志渲染缓存：只在日志条数变化时重新拷贝。
    /// 原来每帧都 `log::recent()` 克隆 60 条（含 String），传输过程中日志不变时纯属浪费。
    log_cache: Vec<crate::log::LogEntry>,
    log_cache_len: usize,

    ak: String,
    sk: String,
    cfg_path: PathBuf,

    up_path: String,
    up_key: String,
    up_ext: String,
    dl_key: String,
    dl_out: String,
    msg: String,

    history: Vec<HistoryItem>,
    /// 历史记录异步拉取：绝不能在 UI 线程里做网络请求，否则界面会卡住
    history_loading: Arc<AtomicBool>,
    history_pending: Arc<Mutex<Option<Vec<HistoryItem>>>>,
    /// 需要重新拉取历史。由**上传线程在写回记录成功后**置位，
    /// 这样能保证「先写完再拉取」的时序；UI 只负责取这个标记并触发。
    history_refresh: Arc<AtomicBool>,
}

impl App {
    fn new(mut cfg: Config, sections: Sections, font_used: Option<String>) -> App {
        let up_ext = cfg.file_ext.clone().unwrap_or_default();
        // 环境段只认 ini 里真实存在的 section —— 不再硬塞默认值，
        // 否则没配 [outer] 也会在下拉框里看到它
        let mut envs: Vec<String> = sections
            .keys()
            .filter(|k| k.as_str() != "common")
            .cloned()
            .collect();
        envs.sort();

        // 环境优先级：上次使用的 > 配置里的默认 > 第一个环境段
        let mut env = cfg.env.clone();
        if let Some(last) = load_last_env() {
            if envs.contains(&last) {
                env = last;
            }
        }
        if envs.is_empty() {
            env = cfg.env.clone();
        } else if !envs.contains(&env) {
            env = envs.first().cloned().unwrap_or_default();
        }
        // 用最终环境重建配置（base_url 等取自对应 section）
        if !env.is_empty() {
            let opts = vec![("--env".to_string(), env.clone())];
            if let Ok(c) = config::resolve_sections_relaxed(&sections, &opts) {
                cfg = c;
            }
        }
        let env = cfg.env.clone();
        let cfg_path = config::default_config_path();
        let mut app = App {
            ak: cfg.access_key.clone(),
            sk: cfg.secret_key.clone(),
            cfg_path,
            cfg,
            sections,
            envs,
            env,
            tasks: Shared::default(),
            next_id: 1,
            logs: log::new_buf(),
            log_auto_scroll: true,
            log_open: false,
            log_cache: Vec::new(),
            log_cache_len: usize::MAX, // 强制首帧填充一次
            up_path: String::new(),
            up_key: String::new(),
            up_ext,
            dl_key: String::new(),
            dl_out: String::new(),
            msg: String::new(),
            history: Vec::new(),
            history_loading: Arc::new(AtomicBool::new(false)),
            history_pending: Arc::new(Mutex::new(None)),
            history_refresh: Arc::new(AtomicBool::new(false)),
        };
        app.load_history_async();
        log::push(&app.logs, "INFO", format!("界面已就绪，当前环境：{}", app.env));
        // 把配置来源也记进日志，便于事后排查「凭据从哪来」
        log::push(&app.logs, "INFO", app.config_source().0);
        // 中文字体加载情况：没加载到时中文会显示成方块，这里明确提示
        match &font_used {
            Some(p) => log::push(&app.logs, "INFO", format!("中文字体：{}", p)),
            None => log::push(
                &app.logs,
                "WARN",
                "未找到系统中文字体，界面中文可能显示为方块".to_string(),
            ),
        }
        app
    }

    fn push_task(&mut self, kind: &str, name: &str, key: &str, out: &str, total: u64) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        if let Ok(mut v) = self.tasks.lock() {
            v.push(TaskState {
                id,
                kind: kind.to_string(),
                name: name.to_string(),
                key: key.to_string(),
                out: out.to_string(),
                total,
                done: 0,
                speed: 0,
                status: "running".to_string(),
                error: String::new(),
                url: String::new(),
            });
        }
        id
    }

    fn start_upload(&mut self) {
        if !self.has_credentials() {
            return;
        }
        let path = PathBuf::from(&self.up_path);
        if !path.exists() {
            self.msg = "文件不存在".to_string();
            return;
        }
        let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        let file_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "file".to_string());
        let key = if self.up_key.trim().is_empty() {
            file_name.clone()
        } else {
            self.up_key.trim().to_string()
        };
        let name = if self.up_ext.trim().is_empty() {
            file_name.clone()
        } else {
            let stem = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| file_name.clone());
            format!("{}.{}", stem, self.up_ext.trim().trim_start_matches('.'))
        };
        let id = self.push_task("上传", &file_name, &key, "", size);

        let cfg = self.cfg.clone();
        let tasks = self.tasks.clone();
        let logs = self.logs.clone();
        let refresh = self.history_refresh.clone();
        let small_limit = cfg.small_file_limit;
        log::push(&self.logs, "INFO", format!("开始上传：{}（key={}）", file_name, key));
        std::thread::spawn(move || {
            let result: Res<String> = (|| {
                let client = Client::new(&cfg, Some(logs.clone()))?;
                let io = Iobs::new(cfg.clone(), client);
                let mut prog = Progress::for_shared(id, tasks.clone());
                let r = if size < small_limit {
                    io.upload_small(&path, &key, &name, &mut prog)?
                } else {
                    io.upload_multipart(&path, &key, &name, &mut prog)?
                };
                // 先把本次上传写入最近记录（等写回 iobs 后再结束，保证历史是最新的）
                let rec = io.record_upload(&key, &name);
                prog.finish();
                match rec {
                    Ok(()) => {
                        // 写回成功，通知界面重新拉取。
                        // 必须由这里置位（而不是界面猜「任务完成了」），
                        // 才能保证「先写完、再拉取」的时序。
                        refresh.store(true, Ordering::SeqCst);
                    }
                    Err(e) => log::push(&logs, "WARN", format!("写入历史记录失败：{}", e)),
                }
                Ok(r)
            })();
            finish_task(&tasks, &logs, id, result);
        });
        self.msg = "已开始上传".to_string();
    }

    fn start_download(&mut self) {
        if !self.has_credentials() {
            return;
        }
        let key = self.dl_key.trim().to_string();
        if key.is_empty() {
            self.msg = "请填写 key".to_string();
            return;
        }
        let out = if self.dl_out.trim().is_empty() {
            PathBuf::from(key.replace('/', "_").replace('\\', "_"))
        } else {
            let p = PathBuf::from(self.dl_out.trim());
            if p.is_dir() || self.dl_out.ends_with('/') || self.dl_out.ends_with('\\') {
                p.join(key.replace('/', "_").replace('\\', "_"))
            } else {
                p
            }
        };
        let out_s = out.to_string_lossy().to_string();
        let id = self.push_task("下载", &key, &key, &out_s, 0);

        let cfg = self.cfg.clone();
        let tasks = self.tasks.clone();
        let logs = self.logs.clone();
        log::push(&self.logs, "INFO", format!("开始下载：{} -> {}", key, out_s));
        std::thread::spawn(move || {
            let result: Res<String> = (|| {
                let client = Client::new(&cfg, Some(logs.clone()))?;
                let io = Iobs::new(cfg.clone(), client);
                let mut prog = Progress::for_shared(id, tasks.clone());
                io.download(&key, &out, false, &mut prog)?;
                prog.finish();
                Ok(String::new())
            })();
            finish_task(&tasks, &logs, id, result);
        });
        self.msg = "已开始下载".to_string();
    }

    /// 异步拉取历史记录。
    ///
    /// 原来这里是同步的 `load_history()`：在 UI 线程里直接发 HTTP 请求，
    /// 一次网络往返几百毫秒，期间界面完全冻结（切环境、点刷新、任务完成时都会触发）。
    /// 改成后台线程拉取，结果放进 `history_pending`，由 `ui()` 每帧取回。
    /// 返回值：true 表示真的发起了请求；false 表示没发起（缺凭据，或上一轮还在飞）。
    /// 调用方据此决定是否保留「待刷新」标记 —— 否则一旦被跳过就再也不会重试。
    fn load_history_async(&mut self) -> bool {
        if self.cfg.access_key.is_empty() || self.cfg.secret_key.is_empty() {
            return false;
        }
        // 上一轮还没回来就跳过，避免重复请求
        if self.history_loading.swap(true, Ordering::SeqCst) {
            return false;
        }
        let cfg = self.cfg.clone();
        let logs = self.logs.clone();
        let pending = self.history_pending.clone();
        let loading = self.history_loading.clone();
        std::thread::spawn(move || {
            // 守卫：即使线程内 panic，也会把 loading 复位，
            // 否则标记永远卡在 true，历史记录就再也不刷新了
            let _guard = LoadingGuard(loading.clone());
            let items = match Client::new(&cfg, Some(logs.clone())) {
                Ok(client) => Iobs::new(cfg, client).load_history(),
                Err(_) => Vec::new(),
            };
            let mut g = match pending.lock() {
                Ok(g) => g,
                // 之前有线程持锁 panic 过 → 恢复数据而不是让界面永远拿不到结果
                Err(poisoned) => poisoned.into_inner(),
            };
            *g = Some(items);
        });
        true
    }

    /// 标记一次历史刷新请求（点「刷新」、切换环境时用）
    fn request_history_refresh(&self) {
        self.history_refresh.store(true, Ordering::SeqCst);
    }

    /// 从历史记录一键下载：保存到 dl_out（留空则当前目录），文件名用记录里的原始文件名
    fn start_download_key(&mut self, key: &str, name: &str) {
        if !self.has_credentials() {
            return;
        }
        // 转成自有字符串，避免把借用借给后台线程导致生命周期逃逸
        let key = key.to_string();
        let name = name.to_string();
        let out_name = if name.trim().is_empty() {
            key.replace('/', "_").replace('\\', "_")
        } else {
            name.clone()
        };
        let out = if self.dl_out.trim().is_empty() {
            PathBuf::from(&out_name)
        } else {
            let p = PathBuf::from(self.dl_out.trim());
            if p.is_dir() || self.dl_out.ends_with('/') || self.dl_out.ends_with('\\') {
                p.join(&out_name)
            } else {
                p
            }
        };
        let out_s = out.to_string_lossy().to_string();
        let id = self.push_task("下载", &out_name, &key, &out_s, 0);
        let cfg = self.cfg.clone();
        let tasks = self.tasks.clone();
        let logs = self.logs.clone();
        log::push(&self.logs, "INFO", format!("从历史下载：{} -> {}", key, out_s));
        std::thread::spawn(move || {
            let result: Res<String> = (|| {
                let client = Client::new(&cfg, Some(logs.clone()))?;
                let io = Iobs::new(cfg.clone(), client);
                let mut prog = Progress::for_shared(id, tasks.clone());
                io.download(&key, &out, false, &mut prog)?;
                prog.finish();
                Ok(String::new())
            })();
            finish_task(&tasks, &logs, id, result);
        });
    }

    fn has_credentials(&mut self) -> bool {
        // 界面里刚填但还没「保存」的凭据也能直接用
        if self.cfg.access_key.is_empty() && !self.ak.trim().is_empty() {
            self.cfg.access_key = self.ak.trim().to_string();
        }
        if self.cfg.secret_key.is_empty() && !self.sk.trim().is_empty() {
            self.cfg.secret_key = self.sk.trim().to_string();
        }
        if self.cfg.access_key.is_empty() || self.cfg.secret_key.is_empty() {
            self.msg = "请先在上方「连接凭据」填写 access_key / secret_key（可点保存）".to_string();
            return false;
        }
        true
    }

    /// 把界面里的 ak/sk 写回 iobs.ini（落到 common 段，所有环境段都会继承）
    fn save_credentials(&mut self) {
        let ak = self.ak.trim().to_string();
        let sk = self.sk.trim().to_string();
        if ak.is_empty() || sk.is_empty() {
            self.msg = "access_key 与 secret_key 均不能为空".to_string();
            return;
        }
        let common = self.sections.entry("common".to_string()).or_default();
        common.insert("access_key".to_string(), ak.clone());
        common.insert("secret_key".to_string(), sk.clone());
        match config::write_sections(&self.cfg_path, &self.sections) {
            Ok(()) => {
                let opts = vec![("--env".to_string(), self.env.clone())];
                match config::resolve_sections_relaxed(&self.sections, &opts) {
                    Ok(c) => {
                        self.cfg = c;
                        self.msg = format!("已保存到 {}", self.cfg_path.display());
                        log::push(&self.logs, "INFO", format!("凭据已保存到 {}", self.cfg_path.display()));
                    }
                    Err(e) => {
                        self.msg = format!("刷新配置失败：{}", e);
                        log::push(&self.logs, "ERR", format!("刷新配置失败：{}", e));
                    }
                }
            }
            Err(e) => {
                self.msg = format!("保存失败：{}", e);
                log::push(&self.logs, "ERR", format!("凭据保存失败：{}", e));
            }
        }
    }
}

fn finish_task(tasks: &Shared, logs: &LogBuf, id: u64, result: Res<String>) {
    if let Ok(mut v) = tasks.lock() {
        if let Some(t) = v.iter_mut().find(|t| t.id == id) {
            match result {
                Ok(url) => {
                    t.url = url;
                    if t.status == "running" {
                        t.status = "done".to_string();
                    }
                }
                Err(e) => {
                    t.status = "error".to_string();
                    t.error = e.to_string();
                }
            }
            let (kind, name, status, err) =
                (t.kind.clone(), t.name.clone(), t.status.clone(), t.error.clone());
            if status == "error" {
                crate::log::push(logs, "ERR", format!("[{}] {} 失败：{}", kind, name, err));
            } else if status == "done" {
                crate::log::push(logs, "OK", format!("[{}] {} 完成", kind, name));
            }
        }
    }
}

impl App {
    /// 「配置来源」说明：当前用的是哪个 iobs.ini、凭据从哪来。
    /// 目的是让用户一眼看清凭据的来源——否则把 exe 放进已有 iobs.ini 的目录时，
    /// 会出现「没填 ak/sk 却能用」的困惑（凭据被同名 ini 静默带入）。
    fn config_source(&self) -> (String, egui::Color32) {
        let has_cred = !self.cfg.access_key.is_empty() && !self.cfg.secret_key.is_empty();
        let ok = egui::Color32::from_rgb(22, 163, 74);
        let warn = egui::Color32::from_rgb(217, 119, 6);
        let err = egui::Color32::from_rgb(220, 38, 38);
        match &self.cfg.config_path {
            Some(p) => {
                let path = p.display();
                if has_cred {
                    let how = if self.cfg.config_explicit {
                        "--config 指定"
                    } else {
                        "按查找顺序命中"
                    };
                    (
                        format!("配置：{}（{}，凭据读自该文件）", path, how),
                        ok,
                    )
                } else {
                    (format!("配置：{}（该文件里没有凭据）", path), warn)
                }
            }
            None => {
                if has_cred {
                    (
                        "配置：未找到 iobs.ini（凭据来自命令行或环境变量）".to_string(),
                        warn,
                    )
                } else {
                    (
                        format!(
                            "配置：未找到 iobs.ini，使用内置默认值；保存将写入 {}",
                            self.cfg_path.display()
                        ),
                        err,
                    )
                }
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        // 取回后台线程拉好的历史记录
        {
            let mut g = match self.history_pending.lock() {
                Ok(g) => g,
                Err(poisoned) => poisoned.into_inner(),
            };
            if let Some(items) = g.take() {
                self.history = items;
            }
        }

        // 需要刷新（上传线程写回成功后置位 / 点刷新 / 切环境）就发起一次异步拉取。
        // 若此刻正好有请求在飞，load_history_async 会返回 false，
        // 那就把标记放回去，下一帧再试 —— 不能像之前那样乐观地认为已经刷新过了。
        if self.history_refresh.swap(false, Ordering::SeqCst) {
            if !self.load_history_async() {
                // 只有当「有请求在飞」时才把标记放回去等下一帧；
                // 若是因为没配凭据而失败，就丢弃，避免标记永远挂着
                if self.history_loading.load(Ordering::SeqCst) {
                    self.history_refresh.store(true, Ordering::SeqCst);
                }
            }
        }

        // 有任务在跑才持续重绘。100ms（10fps）对进度条已足够，
        // 再快只会让低配机器白烧 CPU —— 每帧都要重建整个界面。
        let running = self
            .tasks
            .lock()
            .map(|v| v.iter().any(|t| t.status == "running"))
            .unwrap_or(false);
        if running {
            ctx.request_repaint_after(Duration::from_millis(100));
        }

        egui::containers::panel::CentralPanel::default().show(ui, |ui| {
            // 三分区布局（高度写死，不依赖窗口大小）：
            //   顶栏（固定）→ 表单区（占剩余，可滚动）→ 底部任务/日志（固定，始终可见）
            let avail = ui.available_size();
            let header_h = 62.0;
            let dock_h = (avail.y * 0.38).clamp(150.0, 360.0);
            let forms_h = (avail.y - header_h - dock_h - 16.0).max(80.0);

            self.draw_header(ui, egui::vec2(avail.x, header_h));

            ui.separator();

            egui::ScrollArea::vertical()
                .id_salt("forms_scroll")
                .max_height(forms_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    self.draw_forms(ui);
                });

            ui.separator();

            ui.allocate_ui(egui::vec2(avail.x, dock_h), |ui| {
                self.draw_dock(ui, &ctx);
            });
        });
    }
}

impl App {
    /// 顶栏：标题 + 环境切换 + 配置来源
    fn draw_header(&mut self, ui: &mut egui::Ui, size: egui::Vec2) {
        ui.allocate_ui(size, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("sync-iobs").size(18.0).color(TEXT).strong());
                ui.add_space(2.0);
                ui.label(
                    egui::RichText::new("iobs 文件双向同步").color(TEXT_MUTED).small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if self.envs.is_empty() {
                        ui.label(
                            egui::RichText::new("ini 中未配置环境段")
                                .color(WARN)
                                .small(),
                        );
                        return;
                    }
                    let mut changed = false;
                    egui::ComboBox::from_id_salt("env")
                        .selected_text(format!("环境：{}", self.env))
                        .show_ui(ui, |ui| {
                            for e in self.envs.clone() {
                                if ui.selectable_value(&mut self.env, e.clone(), &e).clicked() {
                                    changed = true;
                                }
                            }
                        });
                    if changed {
                        save_last_env(&self.env);
                        let opts = vec![("--env".to_string(), self.env.clone())];
                        match config::resolve_sections_relaxed(&self.sections, &opts) {
                            Ok(c) => {
                                self.ak = c.access_key.clone();
                                self.sk = c.secret_key.clone();
                                self.cfg = c;
                                self.msg = format!("已切换到 {}", self.env);
                                log::push(&self.logs, "INFO", format!("已切换环境：{} -> {}", self.env, self.cfg.base_url));
                                self.request_history_refresh();
                            }
                            Err(e) => {
                                self.msg = format!("切换失败：{}", e);
                                log::push(&self.logs, "ERR", format!("切换环境失败：{}", e));
                            }
                        }
                    }
                });
            });

            // 配置来源：如实告知用的是哪个 ini、凭据从哪来
            let (src_text, src_color) = self.config_source();
            ui.label(egui::RichText::new(src_text).color(src_color).small());
        });
    }

    /// 中部表单：连接凭据 / 上传 / 下载 / 最近上传（放在 ScrollArea 里，不会把下面的内容挤出屏幕）
    fn draw_forms(&mut self, ui: &mut egui::Ui) {
        ui.spacing_mut().item_spacing.y = 12.0;

        // —— 连接凭据 ——
        let missing_cred = self.cfg.access_key.is_empty() || self.cfg.secret_key.is_empty();
        card(ui, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, "连接凭据");
                if missing_cred {
                    chip(ui, "未配置", DANGER, DANGER_SOFT);
                } else {
                    chip(ui, "已配置", OK, OK_SOFT);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(ui, "保存") {
                        self.save_credentials();
                    }
                });
            });
            ui.add_space(2.0);
            form_row(ui, "AK", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.ak)
                        .hint_text("access_key")
                        .password(true)
                        .desired_width(300.0),
                );
            });
            form_row(ui, "SK", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.sk)
                        .hint_text("secret_key")
                        .password(true)
                        .desired_width(300.0),
                );
            });
            ui.label(
                egui::RichText::new(format!("保存位置：{}", self.cfg_path.display()))
                    .color(TEXT_FAINT)
                    .small(),
            );
        });

        // —— 上传 ——
        card(ui, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, "上传");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "开始上传") {
                        self.start_upload();
                    }
                });
            });
            ui.add_space(2.0);
            form_row(ui, "文件", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.up_path)
                        .hint_text("选择本地文件")
                        .desired_width(300.0),
                );
                if ghost_button(ui, "浏览") {
                    if let Some(p) = rfd::FileDialog::new().pick_file() {
                        self.up_path = p.to_string_lossy().to_string();
                        if self.up_key.trim().is_empty() {
                            self.up_key = p
                                .file_name()
                                .map(|s| s.to_string_lossy().to_string())
                                .unwrap_or_default();
                        }
                    }
                }
            });
            form_row(ui, "key", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.up_key)
                        .hint_text("留空则用文件名")
                        .desired_width(140.0),
                );
                ui.add_space(4.0);
                ui.label(egui::RichText::new("后缀").color(TEXT_MUTED).small());
                ui.add(
                    egui::TextEdit::singleline(&mut self.up_ext)
                        .hint_text("留空保持原样")
                        .desired_width(90.0),
                );
            });
        });

        // —— 下载 ——
        card(ui, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, "下载");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if primary_button(ui, "开始下载") {
                        self.start_download();
                    }
                });
            });
            ui.add_space(2.0);
            form_row(ui, "key", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.dl_key)
                        .hint_text("iobs 中的 key")
                        .desired_width(300.0),
                );
            });
            form_row(ui, "保存", |ui| {
                ui.add(
                    egui::TextEdit::singleline(&mut self.dl_out)
                        .hint_text("留空则保存到当前目录")
                        .desired_width(300.0),
                );
                if ghost_button(ui, "选择目录") {
                    if let Some(p) = rfd::FileDialog::new().pick_folder() {
                        self.dl_out = p.to_string_lossy().to_string();
                    }
                }
            });
        });

        // —— 最近上传 ——
        card(ui, |ui| {
            ui.horizontal(|ui| {
                section_title(ui, "最近上传");
                ui.label(
                    egui::RichText::new(format!("存于 iobs key: {}", HISTORY_KEY))
                        .color(TEXT_FAINT)
                        .small(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ghost_button(ui, "刷新") {
                        self.request_history_refresh();
                    }
                });
            });
            ui.add_space(2.0);
            if self.history.is_empty() {
                ui.label(
                    egui::RichText::new("暂无记录（上传后会自动记录）")
                        .color(TEXT_FAINT)
                        .small(),
                );
            } else {
                let items: Vec<(String, String, u64)> = self
                    .history
                    .iter()
                    .map(|it| (it.name.clone(), it.key.clone(), it.time))
                    .collect();
                for (name, key, time) in items {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(&name).color(TEXT).strong());
                        if key != name {
                            ui.label(egui::RichText::new(&key).color(TEXT_FAINT).small());
                        }
                        ui.label(
                            egui::RichText::new(fmt_time(time)).color(TEXT_FAINT).small(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ghost_button(ui, "下载") {
                                self.start_download_key(&key, &name);
                            }
                        });
                    });
                    ui.add_space(2.0);
                }
            }
        });

        ui.add_space(4.0);
    }

    /// 底部停靠区：任务进度 + 操作日志（高度固定，始终可见，不会滚出屏幕）
    fn draw_dock(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let avail_h = ui.available_height();
        // 日志默认折叠：折叠时任务区吃满剩余空间；展开后再按比例分配
        let log_h = if self.log_open { (avail_h * 0.4).max(60.0) } else { 0.0 };
        let tasks_h = (avail_h - log_h - 30.0).max(60.0);

        // ---- 任务区 ----
        ui.horizontal(|ui| {
            section_title(ui, "任务");
            if !self.msg.is_empty() {
                ui.label(egui::RichText::new(&self.msg).color(TEXT_MUTED).small());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ghost_button(ui, "清空已完成") {
                    if let Ok(mut v) = self.tasks.lock() {
                        v.retain(|t| t.status == "running");
                    }
                }
            });
        });

        let tasks: Vec<TaskState> = self.tasks.lock().map(|v| v.clone()).unwrap_or_default();
        if tasks.is_empty() {
            ui.label(egui::RichText::new("暂无任务").color(TEXT_FAINT).small());
        } else {
            egui::ScrollArea::vertical()
                .id_salt("tasks_scroll")
                .max_height(tasks_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    // 每个任务组含进度条/明细行，是最重的部件；只渲染最近 12 条
                    for i in 0..tasks.len().min(12) {
                        let t = tasks[tasks.len() - 1 - i].clone();
                        self.draw_task(ui, ctx, &t);
                    }
                });
        }

        ui.add_space(2.0);
        ui.separator();

        // ---- 操作日志（默认折叠，点标题栏展开）----
        let entry_count = self.logs.lock().map(|v| v.len()).unwrap_or(0);
        let state = egui::collapsing_header::CollapsingState::load_with_default_open(
            ui.ctx(),
            ui.id().with("log_section"),
            false,
        );
        let header = state.show_header(ui, |ui| {
            section_title(ui, "操作日志");
            ui.label(
                egui::RichText::new(format!("{} 条", entry_count))
                    .color(TEXT_FAINT)
                    .small(),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ghost_button(ui, "清空") {
                    log::clear(&self.logs);
                }
                ui.checkbox(&mut self.log_auto_scroll, "自动滚动");
            });
        });
        self.log_open = header.is_open();
        if !self.log_open {
            return;
        }
        header.body_unindented(|ui| {
            // 日志条数没变就复用缓存，避免每帧克隆几十条（含 String）
            let cur_len = self.logs.lock().map(|v| v.len()).unwrap_or(0);
            if cur_len != self.log_cache_len {
                self.log_cache = log::recent(&self.logs, 300);
                self.log_cache_len = cur_len;
            }

            if self.log_cache.is_empty() {
                ui.label(egui::RichText::new("暂无日志").color(TEXT_FAINT).small());
                return;
            }

            // 虚拟化：只构建视口内可见的行，滚到哪渲染到哪
            let row_h = 15.0;
            let total = self.log_cache.len();
            let scroll = egui::ScrollArea::vertical()
                .id_salt("log_scroll")
                .max_height(log_h)
                .auto_shrink([false, false]);
            let scroll = if self.log_auto_scroll {
                scroll.stick_to_bottom(true)
            } else {
                scroll
            };
            let cache = &self.log_cache;
            scroll.show_rows(ui, row_h, total, |ui, range| {
                for idx in range {
                    if let Some(e) = cache.get(idx) {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            ui.label(
                                egui::RichText::new(fmt_hms(e.t))
                                    .monospace()
                                    .color(TEXT_FAINT)
                                    .small(),
                            );
                            let (color, tag) = log_style(e.level);
                            ui.label(
                                egui::RichText::new(format!("[{}]", tag))
                                    .color(color)
                                    .monospace()
                                    .small()
                                    .strong(),
                            );
                            ui.label(
                                egui::RichText::new(&e.msg).monospace().color(TEXT).small(),
                            );
                        });
                    }
                }
            });
        });
    }
}

impl App {
    /// 单个任务的渲染（上传/下载通用）
    fn draw_task(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &TaskState) {
        card(ui, |ui| {
            ui.horizontal(|ui| {
                let (fg, bg, kind) = if t.kind == "上传" {
                    (ACCENT, ACCENT_SOFT, "上传")
                } else {
                    (OK, OK_SOFT, "下载")
                };
                chip(ui, kind, fg, bg);
                ui.label(egui::RichText::new(&t.name).color(TEXT).strong());
                if !t.key.is_empty() && t.key != t.name {
                    ui.label(egui::RichText::new(&t.key).color(TEXT_FAINT).small());
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match t.status.as_str() {
                        "running" => chip(ui, "进行中", ACCENT, ACCENT_SOFT),
                        "done" => chip(ui, "完成", OK, OK_SOFT),
                        _ => chip(ui, "失败", DANGER, DANGER_SOFT),
                    }
                });
            });

            if t.status == "error" {
                ui.label(egui::RichText::new(&t.error).color(DANGER).small());
            } else {
                let frac = if t.total > 0 {
                    (t.done as f32 / t.total as f32).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let bar_color = if t.kind == "上传" { ACCENT } else { OK };
                // 细进度条 + 单独的明细行（百分比不再叠在条上，更清爽）
                ui.add(
                    egui::ProgressBar::new(frac)
                        .desired_height(8.0)
                        .fill(bar_color),
                );
                let mut detail = if t.total > 0 {
                    format!(
                        "{}% · {} / {} · {}/s",
                        (frac * 100.0) as u32,
                        human_bytes(t.done),
                        human_bytes(t.total),
                        human_bytes(t.speed)
                    )
                } else {
                    format!(
                        "已接收 {} · {}/s",
                        human_bytes(t.done),
                        human_bytes(t.speed)
                    )
                };
                if t.total > 0 && t.done < t.total && t.speed > 0 {
                    let left = (t.total - t.done) as f64;
                    let eta = (left / t.speed as f64) as u64;
                    detail.push_str(&format!(" · 剩余 {}s", eta));
                }
                ui.label(egui::RichText::new(detail).color(TEXT_MUTED).small());
            }

            ui.horizontal(|ui| {
                if !t.url.is_empty() {
                    if ghost_button(ui, "复制地址") {
                        ctx.copy_text(t.url.clone());
                        self.msg = "地址已复制到剪贴板".to_string();
                    }
                }
                if !t.out.is_empty() && (t.status == "done") {
                    if ghost_button(ui, "打开目录") {
                        let p = t.out.clone();
                        std::thread::spawn(move || {
                            reveal_in_file_manager(&p);
                        });
                    }
                }
            });
        });
    }
}

/// 保证 `history_loading` 一定会被复位（Drop 在线程结束或 panic 时都会执行）
struct LoadingGuard(Arc<AtomicBool>);
impl Drop for LoadingGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::SeqCst);
    }
}

/// GUI 状态文件路径（与 iobs.ini 同目录）
///
/// 刻意**不写回 iobs.ini**：`write_sections` 会按解析结果重建整个文件，
/// 用户手写的注释会被抹掉。状态单独存一个小文件最安全。
fn state_path() -> PathBuf {
    let mut p = config::default_config_path();
    p.set_file_name("sync-iobs.state");
    p
}

/// 读取上次使用的环境名
fn load_last_env() -> Option<String> {
    let text = std::fs::read_to_string(state_path()).ok()?;
    let v = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("last_env="))
        .map(|s| s.trim().to_string())?;
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// 记住本次选择的环境（写失败也不影响使用，静默忽略）
fn save_last_env(env: &str) {
    if env.is_empty() {
        return;
    }
    let _ = std::fs::write(state_path(), format!("last_env={}\n", env));
}

/// 在系统文件管理器里定位文件（各系统命令不同；原先只写了 explorer，macOS 上点了没反应）
fn reveal_in_file_manager(path: &str) {
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer")
        .args(["/select,", path])
        .spawn();
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").args(["-R", path]).spawn();
    #[cfg(all(not(windows), not(target_os = "macos")))]
    let _ = std::process::Command::new("xdg-open")
        .arg(
            std::path::Path::new(path)
                .parent()
                .unwrap_or(std::path::Path::new(".")),
        )
        .spawn();
}

// GUI 模式下把控制台窗口藏起来（双击 exe 时不会闪黑框）
#[cfg(windows)]
extern "system" {
    fn GetConsoleWindow() -> *mut std::ffi::c_void;
    fn ShowWindow(hwnd: *mut std::ffi::c_void, n_cmd_show: i32) -> i32;
}

fn hide_console() {
    #[cfg(windows)]
    unsafe {
        let hwnd = GetConsoleWindow();
        if !hwnd.is_null() {
            ShowWindow(hwnd, 0);
        }
    }
}

/// 把 Unix 秒格式化成 YYYY-MM-DD HH:MM:SS（按 UTC 显示）
fn fmt_time(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let (y, m, d) = civil_from_days(days);
    let sod = secs % 86400;
    let hh = sod / 3600;
    let mm = (sod % 3600) / 60;
    let ss = sod % 60;
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, m, d, hh, mm, ss)
}

/// 日志行只显示时分秒，节省横向空间
fn fmt_hms(secs: u64) -> String {
    let sod = secs % 86400;
    format!("{:02}:{:02}:{:02}", sod / 3600, (sod % 3600) / 60, sod % 60)
}

/// 日志级别 -> (颜色, 标签)
fn log_style(level: &str) -> (egui::Color32, &'static str) {
    match level {
        "OK" => (OK, "OK  "),
        "WARN" => (WARN, "WARN"),
        "ERR" => (DANGER, "ERR "),
        _ => (egui::Color32::from_rgb(100, 116, 139), "INFO"),
    }
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
