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

const ACCENT: egui::Color32 = egui::Color32::from_rgb(37, 99, 235);

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
    let r = egui::CornerRadius::same(6);
    style.visuals.widgets.noninteractive.corner_radius = r;
    style.visuals.widgets.inactive.corner_radius = r;
    style.visuals.widgets.hovered.corner_radius = r;
    style.visuals.widgets.active.corner_radius = r;
    style.visuals.widgets.inactive.weak_bg_fill = egui::Color32::from_rgb(243, 244, 246);
    style.spacing.item_spacing = egui::vec2(8.0, 7.0);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    ctx.set_style_of(egui::Theme::Light, style);
    ctx.set_theme(egui::Theme::Light);
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
    history_done: u64,
    /// 历史记录异步拉取：绝不能在 UI 线程里做网络请求，否则界面会卡住
    history_loading: Arc<AtomicBool>,
    history_pending: Arc<Mutex<Option<Vec<HistoryItem>>>>,
}

impl App {
    fn new(cfg: Config, sections: Sections, font_used: Option<String>) -> App {
        let env = cfg.env.clone();
        let up_ext = cfg.file_ext.clone().unwrap_or_default();
        let mut envs: Vec<String> = sections
            .keys()
            .filter(|k| k.as_str() != "common")
            .cloned()
            .collect();
        envs.sort();
        if !envs.contains(&env) {
            envs.push(env.clone());
        }
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
            up_path: String::new(),
            up_key: String::new(),
            up_ext,
            dl_key: String::new(),
            dl_out: String::new(),
            msg: String::new(),
            history: Vec::new(),
            history_done: 0,
            history_loading: Arc::new(AtomicBool::new(false)),
            history_pending: Arc::new(Mutex::new(None)),
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
                if let Err(e) = rec {
                    let _ = e;
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
    fn load_history_async(&mut self) {
        if self.cfg.access_key.is_empty() || self.cfg.secret_key.is_empty() {
            return;
        }
        // 上一轮还没回来就跳过，避免重复请求
        if self.history_loading.swap(true, Ordering::SeqCst) {
            return;
        }
        let cfg = self.cfg.clone();
        let logs = self.logs.clone();
        let pending = self.history_pending.clone();
        let loading = self.history_loading.clone();
        std::thread::spawn(move || {
            let items = match Client::new(&cfg, Some(logs.clone())) {
                Ok(client) => Iobs::new(cfg, client).load_history(),
                Err(_) => Vec::new(),
            };
            if let Ok(mut g) = pending.lock() {
                *g = Some(items);
            }
            loading.store(false, Ordering::SeqCst);
        });
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
        if let Ok(mut g) = self.history_pending.lock() {
            if let Some(items) = g.take() {
                self.history = items;
            }
        }

        // 有上传任务完成时自动刷新历史（走异步，不阻塞界面）
        {
            let done = self
                .tasks
                .lock()
                .map(|v| v.iter().filter(|t| t.status == "done").count() as u64)
                .unwrap_or(0);
            if done > self.history_done {
                self.history_done = done;
                self.load_history_async();
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
            // 顶栏：标题 + 环境切换
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("sync-iobs").color(ACCENT).strong(),
                );
                ui.label(egui::RichText::new("iobs 文件双向同步").weak());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
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
                        let opts = vec![("--env".to_string(), self.env.clone())];
                        match config::resolve_sections_relaxed(&self.sections, &opts) {
                            Ok(c) => {
                                self.ak = c.access_key.clone();
                                self.sk = c.secret_key.clone();
                                self.cfg = c;
                                self.msg = format!("已切换到 {}", self.env);
                                log::push(&self.logs, "INFO", format!("已切换环境：{} -> {}", self.env, self.cfg.base_url));
                                self.load_history_async();
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
        {
            // 连接凭据：缺了也能进界面，在这里补上并保存
            let missing_cred = self.cfg.access_key.is_empty() || self.cfg.secret_key.is_empty();
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("连接凭据").strong());
                        if missing_cred {
                            ui.label(
                                egui::RichText::new("⚠ 未配置 access_key / secret_key")
                                    .color(egui::Color32::from_rgb(220, 38, 38)),
                            );
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("保存").clicked() {
                                self.save_credentials();
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        ui.label("AK  ：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.ak)
                                .hint_text("access_key")
                                .password(true)
                                .desired_width(320.0),
                        );
                        ui.label("SK  ：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.sk)
                                .hint_text("secret_key")
                                .password(true)
                                .desired_width(320.0),
                        );
                    });
                    ui.label(
                        egui::RichText::new(format!("保存位置：{}", self.cfg_path.display()))
                            .weak()
                            .small(),
                    );
                });
            });

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("上传").strong());
                    ui.horizontal(|ui| {
                        ui.label("文件：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.up_path)
                                .hint_text("选择本地文件")
                                .desired_width(340.0),
                        );
                        if ui.button("浏览…").clicked() {
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
                    ui.horizontal(|ui| {
                        ui.label("key ：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.up_key)
                                .hint_text("留空则用文件名")
                                .desired_width(200.0),
                        );
                        ui.label("后缀：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.up_ext)
                                .hint_text("留空保持原样")
                                .desired_width(90.0),
                        );
                        ui.add_space(6.0);
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("开始上传").color(egui::Color32::WHITE),
                            ).fill(ACCENT))
                            .clicked()
                        {
                            self.start_upload();
                        }
                    });
                });
            });

            ui.add_space(8.0);

            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.label(egui::RichText::new("下载").strong());
                    ui.horizontal(|ui| {
                        ui.label("key ：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.dl_key)
                                .hint_text("iobs 中的 key")
                                .desired_width(340.0),
                        );
                    });
                    ui.horizontal(|ui| {
                        ui.label("保存：");
                        ui.add(
                            egui::TextEdit::singleline(&mut self.dl_out)
                                .hint_text("留空则保存到当前目录")
                                .desired_width(340.0),
                        );
                        if ui.button("选择目录…").clicked() {
                            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                                self.dl_out = p.to_string_lossy().to_string();
                            }
                        }
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("开始下载").color(egui::Color32::WHITE),
                            ).fill(ACCENT))
                            .clicked()
                        {
                            self.start_download();
                        }
                    });
                });
            });

            ui.add_space(8.0);

            // 最近上传：存于 iobs 固定 key，可一键快速下载
            ui.group(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new("最近上传").strong());
                        ui.label(
                            egui::RichText::new(format!("(存于 iobs key: {})", HISTORY_KEY)).weak(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("刷新").clicked() {
                                self.load_history_async();
                            }
                        });
                    });
                    if self.history.is_empty() {
                        ui.label(egui::RichText::new("暂无记录（上传后会自动记录）").weak());
                    } else {
                        // 先 clone 出本次要展示的数据，避免在循环里既借 immutable 又借 mutable
                        let items: Vec<(String, String, u64)> = self
                            .history
                            .iter()
                            .map(|it| (it.name.clone(), it.key.clone(), it.time))
                            .collect();
                        for (name, key, time) in items {
                            ui.horizontal(|ui| {
                                ui.label(egui::RichText::new(&name).strong());
                                ui.label(egui::RichText::new(format!("key: {}", key)).weak());
                                ui.label(egui::RichText::new(fmt_time(time)).weak());
                                if ui.button("下载").clicked() {
                                    self.start_download_key(&key, &name);
                                }
                            });
                        }
                    }
                });
            });

            ui.add_space(8.0);
        }
    }

    /// 底部停靠区：任务进度 + 操作日志（高度固定，始终可见，不会滚出屏幕）
    fn draw_dock(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        let avail_h = ui.available_height();
        // 任务区略多于日志区，各自独立滚动；26 是两处标题栏 + 间距的预留
        let tasks_h = (avail_h * 0.55).max(60.0);
        let log_h = (avail_h - tasks_h - 26.0).max(60.0);

        // ---- 任务区 ----
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("任务").strong());
            if !self.msg.is_empty() {
                ui.label(egui::RichText::new(&self.msg).weak());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("清空已完成").clicked() {
                    if let Ok(mut v) = self.tasks.lock() {
                        v.retain(|t| t.status == "running");
                    }
                    // 清空后重新允许「任务完成 → 刷新历史」触发一次
                    self.history_done = 0;
                }
            });
        });

        let tasks: Vec<TaskState> = self.tasks.lock().map(|v| v.clone()).unwrap_or_default();
        if tasks.is_empty() {
            ui.label(egui::RichText::new("暂无任务").weak());
        } else {
            egui::ScrollArea::vertical()
                .id_salt("tasks_scroll")
                .max_height(tasks_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for t in tasks.iter().rev().take(20) {
                        self.draw_task(ui, ctx, t);
                        ui.add_space(4.0);
                    }
                });
        }

        ui.separator();

        // ---- 操作日志 ----
        let entry_count = self.logs.lock().map(|v| v.len()).unwrap_or(0);
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("操作日志").strong());
            ui.label(egui::RichText::new(format!("({} 条)", entry_count)).weak());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("清空日志").clicked() {
                    log::clear(&self.logs);
                }
                ui.checkbox(&mut self.log_auto_scroll, "自动滚动");
            });
        });

        // 只渲染最近 60 条：每条 3 个 label，200 条时低配机器每帧要算 600 个文本，很吃 CPU
        let entries = log::recent(&self.logs, 60);
        if entries.is_empty() {
            ui.label(egui::RichText::new("暂无日志").weak());
        } else {
            let scroll = egui::ScrollArea::vertical()
                .id_salt("log_scroll")
                .max_height(log_h)
                .auto_shrink([false, false]);
            let scroll = if self.log_auto_scroll { scroll.stick_to_bottom(true) } else { scroll };
            scroll.show(ui, |ui| {
                for e in &entries {
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(
                            egui::RichText::new(fmt_hms(e.t))
                                .monospace()
                                .weak()
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
                        ui.label(egui::RichText::new(&e.msg).monospace().small());
                    });
                }
            });
        }
    }
}

impl App {
    /// 单个任务的渲染（上传/下载通用）
    fn draw_task(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, t: &TaskState) {
        ui.group(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    let badge = match t.kind.as_str() {
                        "上传" => egui::RichText::new("上传").color(ACCENT),
                        _ => egui::RichText::new("下载").color(egui::Color32::from_rgb(22, 163, 74)),
                    };
                    ui.label(egui::RichText::new(format!("[{}]", badge.text())).color(ACCENT));
                    ui.label(egui::RichText::new(&t.name).strong());
                    if !t.key.is_empty() && t.key != t.name {
                        ui.label(egui::RichText::new(format!("key: {}", t.key)).weak());
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match t.status.as_str() {
                            "running" => {
                                ui.label(egui::RichText::new("进行中").color(ACCENT));
                            }
                            "done" => {
                                ui.label(
                                    egui::RichText::new("完成")
                                        .color(egui::Color32::from_rgb(22, 163, 74)),
                                );
                            }
                            _ => {
                                ui.label(
                                    egui::RichText::new("失败")
                                        .color(egui::Color32::from_rgb(220, 38, 38)),
                                );
                            }
                        }
                    });
                });

                if t.status == "error" {
                    ui.colored_label(
                        egui::Color32::from_rgb(220, 38, 38),
                        egui::RichText::new(&t.error).small(),
                    );
                } else {
                    let frac = if t.total > 0 {
                        (t.done as f32 / t.total as f32).clamp(0.0, 1.0)
                    } else {
                        0.0
                    };
                    let pct = if t.total > 0 {
                        (frac * 100.0) as u32
                    } else {
                        0
                    };
                    let bar_color = match t.kind.as_str() {
                        "上传" => ACCENT,
                        _ => egui::Color32::from_rgb(22, 163, 74),
                    };
                    // 进度条：加粗、上色、撑满宽度，条上直接标百分比
                    ui.add(
                        egui::ProgressBar::new(frac)
                            .desired_height(16.0)
                            .fill(bar_color)
                            .text(if t.total > 0 {
                                format!("{}%", pct)
                            } else {
                                "传输中…".to_string()
                            }),
                    );
                    // 明细行：已传/总量 · 速度 · 剩余秒数
                    let mut detail = if t.total > 0 {
                        format!(
                            "{} / {}  ·  {}/s",
                            human_bytes(t.done),
                            human_bytes(t.total),
                            human_bytes(t.speed)
                        )
                    } else {
                        format!(
                            "已接收 {}  ·  {}/s",
                            human_bytes(t.done),
                            human_bytes(t.speed)
                        )
                    };
                    if t.total > 0 && t.done < t.total && t.speed > 0 {
                        let left = (t.total - t.done) as f64;
                        let eta = (left / t.speed as f64) as u64;
                        detail.push_str(&format!("  ·  剩余 {}s", eta));
                    }
                    ui.label(egui::RichText::new(detail).weak().small());
                }

                ui.horizontal(|ui| {
                    if !t.url.is_empty() {
                        if ui.button("复制下载地址").clicked() {
                            ctx.copy_text(t.url.clone());
                            self.msg = "地址已复制到剪贴板".to_string();
                        }
                    }
                    if !t.out.is_empty() && (t.status == "done") {
                        if ui.button("打开所在目录").clicked() {
                            let p = t.out.clone();
                            std::thread::spawn(move || {
                                reveal_in_file_manager(&p);
                            });
                        }
                    }
                });
            });
        });
    }
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
        "OK" => (egui::Color32::from_rgb(22, 163, 74), "OK  "),
        "WARN" => (egui::Color32::from_rgb(217, 119, 6), "WARN"),
        "ERR" => (egui::Color32::from_rgb(220, 38, 38), "ERR "),
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
