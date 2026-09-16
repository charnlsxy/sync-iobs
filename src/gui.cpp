#include "gui.hpp"

#include "imgui.h"
#include "imgui_impl_glfw.h"
#include "imgui_impl_opengl3.h"
#include "imgui_impl_opengl3_loader.h"
// InputText 的 std::string 重载（CMake 里已把 misc/cpp/imgui_stdlib.cpp 编进来）
#include "misc/cpp/imgui_stdlib.h"

#include <GLFW/glfw3.h>

#include <algorithm>
#include <cstdio>
#include <ctime>
#include <fstream>
#include <map>
#include <sstream>

#ifdef _WIN32
#include <windows.h>
#endif

namespace siobs {
namespace {

// ---------- 任务模型 ----------

enum class TaskStatus { Running, Done, Error };

struct Task {
    std::uint64_t id = 0;
    std::string kind;   // 上传 / 下载
    std::string name;
    std::string key;
    std::string out;    // 下载保存路径
    std::uint64_t total = 0;
    std::uint64_t done = 0;
    TaskStatus status = TaskStatus::Running;
    std::string error;
    std::string url;
};

struct LogLine {
    std::string time;
    std::string level;  // INFO / OK / WARN / ERR
    std::string msg;
};

struct App {
    Sections sections;
    Config cfg;

    std::vector<std::string> envs;
    std::string env;

    std::string ak;
    std::string sk;

    std::string up_path;
    std::string up_key;
    std::string up_ext;
    std::string dl_key;
    std::string dl_out;

    std::string msg;

    // 任务与日志由后台线程写，UI 线程读，用互斥锁保护
    std::mutex mu;
    std::vector<Task> tasks;
    std::vector<LogLine> logs;
    std::uint64_t next_id = 1;

    std::vector<HistoryItem> history;
    std::atomic<bool> history_loading{false};
    std::atomic<bool> history_refresh{false};

    bool log_open = false;
    bool log_auto_scroll = true;
    std::string cfg_path;

    bool quit = false;
};

std::string hhmmss() {
    std::time_t t = std::time(nullptr);
    std::tm tm{};
#if defined(_WIN32)
    localtime_s(&tm, &t);
#else
    localtime_r(&t, &tm);
#endif
    char buf[16];
    std::snprintf(buf, sizeof(buf), "%02d:%02d:%02d", tm.tm_hour, tm.tm_min, tm.tm_sec);
    return buf;
}

std::string fmt_time(std::uint64_t ts) {
    std::time_t t = static_cast<std::time_t>(ts);
    std::tm tm{};
#if defined(_WIN32)
    localtime_s(&tm, &t);
#else
    localtime_r(&t, &tm);
#endif
    char buf[32];
    std::snprintf(buf, sizeof(buf), "%04d-%02d-%02d %02d:%02d", tm.tm_year + 1900,
                  tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min);
    return buf;
}

std::string human_bytes(std::uint64_t n) {
    const char* units[] = {"B", "KB", "MB", "GB", "TB"};
    double v = static_cast<double>(n);
    int i = 0;
    while (v >= 1024.0 && i < 4) {
        v /= 1024.0;
        ++i;
    }
    char buf[32];
    if (i == 0) std::snprintf(buf, sizeof(buf), "%llu B", static_cast<unsigned long long>(n));
    else std::snprintf(buf, sizeof(buf), "%.1f %s", v, units[i]);
    return buf;
}

void push_log(App& app, const std::string& level, const std::string& msg) {
    std::lock_guard<std::mutex> lk(app.mu);
    app.logs.push_back({hhmmss(), level, msg});
    if (app.logs.size() > 2000) {
        app.logs.erase(app.logs.begin(), app.logs.begin() + 500);
    }
}

std::uint64_t add_task(App& app, const std::string& kind, const std::string& name,
                       const std::string& key, const std::string& out,
                       std::uint64_t total) {
    std::lock_guard<std::mutex> lk(app.mu);
    Task t;
    t.id = app.next_id++;
    t.kind = kind;
    t.name = name;
    t.key = key;
    t.out = out;
    t.total = total;
    app.tasks.push_back(t);
    return t.id;
}

// 注意 std::mutex 不可重入：必须先释放锁，再写日志（push_log 内部也会加锁），
// 否则「持锁 → 调 push_log → 再次请求同一把锁」直接死锁。
void finish_task(App& app, Task t, bool ok, const std::string& error) {
    {
        std::lock_guard<std::mutex> lk(app.mu);
        for (auto& x : app.tasks) {
            if (x.id != t.id) continue;
            x.status = ok ? TaskStatus::Done : TaskStatus::Error;
            x.error = error;
            break;
        }
    }
    if (ok) {
        push_log(app, "OK", "[" + t.kind + "] " + t.name + " 完成");
    } else {
        push_log(app, "ERR", "[" + t.kind + "] " + t.name + " 失败：" + error);
    }
}

// ---------- 后台任务 ----------

void start_upload(App& app) {
    std::string path = app.up_path;
    if (path.empty()) {
        app.msg = "请先选择文件";
        return;
    }
    std::ifstream probe(path, std::ios::binary);
    if (!probe) {
        app.msg = "文件不存在";
        return;
    }

    std::string key = app.up_key;
    std::string name = path;
    {
        std::size_t slash = path.find_last_of("/\\");
        if (slash != std::string::npos) name = path.substr(slash + 1);
    }
    if (key.empty()) key = name;

    std::string ext = app.up_ext.empty() ? app.cfg.file_ext : app.up_ext;
    if (!ext.empty()) {
        std::string stem = name;
        std::size_t dot = name.find_last_of('.');
        if (dot != std::string::npos) stem = name.substr(0, dot);
        while (!ext.empty() && ext.front() == '.') ext.erase(ext.begin());
        name = stem + "." + ext;
    }

    std::uint64_t total = 0;
    {
        probe.seekg(0, std::ios::end);
        total = static_cast<std::uint64_t>(probe.tellg());
    }

    Task snapshot;
    snapshot.id = add_task(app, "上传", name, key, "", total);

    Config cfg = app.cfg;
    std::uint64_t task_id = snapshot.id;
    App* p = &app;

    std::thread([p, cfg, path, key, name, task_id]() {
        std::string err = upload_file(cfg, path, key, name,
                                      [p, task_id](std::uint64_t done, std::uint64_t total) {
                                          std::lock_guard<std::mutex> lk(p->mu);
                                          for (auto& t : p->tasks) {
                                              if (t.id == task_id) {
                                                  t.done = done;
                                                  break;
                                              }
                                          }
                                      });
        Task dummy;
        dummy.id = task_id;
        dummy.kind = "上传";
        dummy.name = name;
        finish_task(*p, dummy, err.empty(), err);

        if (err.empty()) {
            // 写回历史成功再通知界面刷新，保证「先写完再拉取」的时序
            std::string rerr = record_upload(cfg, key, name);
            if (!rerr.empty()) push_log(*p, "WARN", "写入历史记录失败：" + rerr);
            p->history_refresh.store(true);
        }
    }).detach();
    app.msg = "已开始上传";
    push_log(app, "INFO", "开始上传：" + name + "（key=" + key + "）");
}

void start_download(App& app, const std::string& key, const std::string& name) {
    if (key.empty()) {
        app.msg = "请填写 key";
        return;
    }
    std::string out = app.dl_out;
    if (out.empty()) {
        out = name.empty() ? key : name;
    } else if (!out.empty() && (out.back() == '/' || out.back() == '\\')) {
        out += name.empty() ? key : name;
    }

    Task snapshot;
    snapshot.id = add_task(app, "下载", name.empty() ? key : name, key, out, 0);

    Config cfg = app.cfg;
    std::uint64_t task_id = snapshot.id;
    App* p = &app;
    std::string out_path = out;

    std::thread([p, cfg, key, out_path, task_id]() {
        std::string err = download_to_file(cfg, key, out_path,
                                           [p, task_id](std::uint64_t done, std::uint64_t) {
                                               std::lock_guard<std::mutex> lk(p->mu);
                                               for (auto& t : p->tasks) {
                                                   if (t.id == task_id) {
                                                       t.done = done;
                                                       break;
                                                   }
                                               }
                                           });
        Task dummy;
        dummy.id = task_id;
        dummy.kind = "下载";
        dummy.name = key;
        finish_task(*p, dummy, err.empty(), err);
    }).detach();
    app.msg = "已开始下载";
    push_log(app, "INFO", "开始下载：" + key);
}

void refresh_history_async(App& app) {
    if (app.history_loading.load()) return;
    if (app.cfg.access_key.empty() || app.cfg.secret_key.empty()) return;
    app.history_loading.store(true);

    Config cfg = app.cfg;
    App* p = &app;
    std::thread([p, cfg]() {
        std::vector<HistoryItem> items = load_history(cfg);
        std::lock_guard<std::mutex> lk(p->mu);
        p->history = items;
        p->history_loading.store(false);
    }).detach();
}

// ---------- 界面绘制 ----------

void draw_header(App& app, float width) {
    ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(8, 8));
    ImGui::Text("sync-iobs");
    ImGui::SameLine();
    ImGui::TextDisabled("iobs 文件双向同步");

    if (!app.envs.empty()) {
        ImGui::SameLine(width - 150);
        ImGui::SetNextItemWidth(140);
        if (ImGui::BeginCombo("##env", ("环境：" + app.env).c_str())) {
            for (const auto& e : app.envs) {
                bool selected = (e == app.env);
                if (ImGui::Selectable(e.c_str(), selected)) {
                    app.env = e;
                    std::vector<std::pair<std::string, std::string>> opts{{"--env", e}};
                    app.cfg = resolve_config(app.sections, opts);
                    app.ak = app.cfg.access_key;
                    app.sk = app.cfg.secret_key;
                    app.msg = "已切换到 " + e;
                    app.history_refresh.store(true);
                }
                if (selected) ImGui::SetItemDefaultFocus();
            }
            ImGui::EndCombo();
        }
    } else {
        ImGui::SameLine(width - 220);
        ImGui::TextColored(ImVec4(0.85f, 0.47f, 0.02f, 1.0f), "ini 中未配置环境段");
    }

    // 配置来源：明确告知 ini 路径与凭据出处
    if (app.cfg_path.empty()) {
        ImGui::TextColored(ImVec4(0.86f, 0.15f, 0.15f, 1.0f),
                           "未找到 iobs.ini（可用 --config 指定）");
    } else {
        bool has_cred = !app.cfg.access_key.empty() && !app.cfg.secret_key.empty();
        ImVec4 color = has_cred ? ImVec4(0.09f, 0.64f, 0.29f, 1.0f)
                                : ImVec4(0.85f, 0.47f, 0.02f, 1.0f);
        std::string tip = "配置：" + app.cfg_path +
                          (has_cred ? "（凭据读自该文件）" : "（该文件里没有凭据）");
        ImGui::TextColored(color, "%s", tip.c_str());
    }
    ImGui::PopStyleVar();
}

void form_row(const char* label, float label_width, const std::function<void()>& body) {
    ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(8, 8));
    ImGui::SetCursorPosX(ImGui::GetCursorPosX() + label_width - ImGui::CalcTextSize(label).x);
    ImGui::TextDisabled("%s", label);
    ImGui::SameLine();
    ImGui::SetCursorPosX(ImGui::GetCursorPosX());
    body();
    ImGui::PopStyleVar();
}

void draw_forms(App& app) {
    const float label_w = 54.0f;

    // ---- 连接凭据 ----
    if (ImGui::CollapsingHeader("连接凭据", ImGuiTreeNodeFlags_DefaultOpen)) {
        bool has_cred = !app.cfg.access_key.empty() && !app.cfg.secret_key.empty();
        ImGui::SameLine();
        if (has_cred) {
            ImGui::TextColored(ImVec4(0.09f, 0.64f, 0.29f, 1.0f), "已配置");
        } else {
            ImGui::TextColored(ImVec4(0.86f, 0.15f, 0.15f, 1.0f), "未配置");
        }

        form_row("AK", label_w, [&] {
            ImGui::SetNextItemWidth(300);
            ImGui::InputText("##ak", &app.ak, ImGuiInputTextFlags_Password);
        });
        form_row("SK", label_w, [&] {
            ImGui::SetNextItemWidth(300);
            ImGui::InputText("##sk", &app.sk, ImGuiInputTextFlags_Password);
        });
        if (ImGui::Button("保存")) {
            app.cfg.access_key = app.ak;
            app.cfg.secret_key = app.sk;
            app.msg = "凭据已更新（下次启动生效）";
            push_log(app, "OK", "凭据已更新");
            app.history_refresh.store(true);
        }
        ImGui::TextDisabled("保存位置：%s", app.cfg_path.c_str());
    }

    // ---- 上传 ----
    if (ImGui::CollapsingHeader("上传", ImGuiTreeNodeFlags_DefaultOpen)) {
        form_row("文件", label_w, [&] {
            ImGui::SetNextItemWidth(300);
            ImGui::InputText("##up_path", &app.up_path);
            ImGui::SameLine();
            if (ImGui::Button("浏览…")) {
                app.msg = "请在上方输入框填写完整路径（或拖拽文件到窗口）";
            }
        });
        form_row("key", label_w, [&] {
            ImGui::SetNextItemWidth(180);
            ImGui::InputText("##up_key", &app.up_key);
            ImGui::SameLine();
            ImGui::TextDisabled("后缀");
            ImGui::SameLine();
            ImGui::SetNextItemWidth(90);
            ImGui::InputText("##up_ext", &app.up_ext);
        });
        if (ImGui::Button("开始上传")) start_upload(app);
    }

    // ---- 下载 ----
    if (ImGui::CollapsingHeader("下载", ImGuiTreeNodeFlags_DefaultOpen)) {
        form_row("key", label_w, [&] {
            ImGui::SetNextItemWidth(300);
            ImGui::InputText("##dl_key", &app.dl_key);
        });
        form_row("保存", label_w, [&] {
            ImGui::SetNextItemWidth(300);
            ImGui::InputText("##dl_out", &app.dl_out);
            ImGui::SameLine();
            if (ImGui::Button("选择目录…")) {
                app.msg = "请在上方输入框填写保存目录";
            }
        });
        if (ImGui::Button("开始下载")) start_download(app, app.dl_key, "");
    }

    // ---- 最近上传 ----
    std::string title = "最近上传（存于 iobs key: " + std::string(kHistoryKey) + "）";
    if (ImGui::CollapsingHeader(title.c_str(), ImGuiTreeNodeFlags_DefaultOpen)) {
        if (app.history.empty()) {
            ImGui::TextDisabled("暂无记录（上传后会自动记录）");
        } else {
            std::vector<HistoryItem> items;
            {
                std::lock_guard<std::mutex> lk(app.mu);
                items = app.history;
            }
            for (const auto& it : items) {
                ImGui::PushID(it.key.c_str());
                ImGui::Text("%s", it.name.c_str());
                ImGui::SameLine();
                ImGui::TextDisabled("%s", fmt_time(it.time).c_str());
                ImGui::SameLine(ImGui::GetContentRegionAvail().x - 60);
                if (ImGui::Button("下载")) {
                    app.dl_key = it.key;
                    start_download(app, it.key, it.name);
                }
                ImGui::PopID();
            }
        }
        if (ImGui::Button("刷新历史")) app.history_refresh.store(true);
    }
}

void draw_task(const Task& t) {
    bool is_upload = (t.kind == "上传");
    ImVec4 kind_color = is_upload ? ImVec4(0.15f, 0.39f, 0.92f, 1.0f)
                                  : ImVec4(0.09f, 0.64f, 0.29f, 1.0f);

    ImGui::PushStyleColor(ImGuiCol_Text, kind_color);
    ImGui::Text("[%s]", t.kind.c_str());
    ImGui::PopStyleColor();
    ImGui::SameLine();
    ImGui::TextUnformatted(t.name.c_str());

    ImGui::SameLine(ImGui::GetContentRegionAvail().x - 50);
    if (t.status == TaskStatus::Running) {
        ImGui::TextColored(ImVec4(0.15f, 0.39f, 0.92f, 1.0f), "进行中");
    } else if (t.status == TaskStatus::Done) {
        ImGui::TextColored(ImVec4(0.09f, 0.64f, 0.29f, 1.0f), "完成");
    } else {
        ImGui::TextColored(ImVec4(0.86f, 0.15f, 0.15f, 1.0f), "失败");
    }

    if (t.status == TaskStatus::Error) {
        ImGui::TextColored(ImVec4(0.86f, 0.15f, 0.15f, 1.0f), "%s", t.error.c_str());
    } else {
        float frac = (t.total > 0) ? static_cast<float>(t.done) / static_cast<float>(t.total)
                                   : 0.0f;
        frac = std::min(1.0f, std::max(0.0f, frac));
        ImGui::PushStyleColor(ImGuiCol_PlotHistogram, kind_color);
        ImGui::ProgressBar(frac, ImVec2(-1, 8), "");
        ImGui::PopStyleColor();
        std::string detail = (t.total > 0)
            ? std::to_string(static_cast<int>(frac * 100)) + "% · " +
                  human_bytes(t.done) + " / " + human_bytes(t.total)
            : "已接收 " + human_bytes(t.done);
        ImGui::TextDisabled("%s", detail.c_str());
    }
    ImGui::Spacing();
}

void draw_dock(App& app) {
    // ---- 任务区 ----
    ImGui::TextUnformatted("任务");
    if (!app.msg.empty()) {
        ImGui::SameLine();
        ImGui::TextDisabled("%s", app.msg.c_str());
    }
    ImGui::SameLine(ImGui::GetContentRegionAvail().x - 80);
    if (ImGui::Button("清空已完成")) {
        std::lock_guard<std::mutex> lk(app.mu);
        app.tasks.erase(std::remove_if(app.tasks.begin(), app.tasks.end(),
                                       [](const Task& t) {
                                           return t.status != TaskStatus::Running;
                                       }),
                        app.tasks.end());
    }

    std::vector<Task> tasks;
    {
        std::lock_guard<std::mutex> lk(app.mu);
        tasks = app.tasks;
    }
    if (tasks.empty()) {
        ImGui::TextDisabled("暂无任务");
    } else {
        // 最多渲染最近 12 条，避免任务多时拖慢
        for (std::size_t i = tasks.size(); i > 0; --i) {
            if (tasks.size() - i >= 12) break;
            draw_task(tasks[i - 1]);
        }
    }

    ImGui::Separator();

    // ---- 操作日志（默认折叠）----
    std::string log_title = "操作日志";
    {
        std::lock_guard<std::mutex> lk(app.mu);
        log_title += "（" + std::to_string(app.logs.size()) + " 条）";
    }
    app.log_open = ImGui::CollapsingHeader(log_title.c_str());
    ImGui::SameLine(ImGui::GetContentRegionAvail().x - 140);
    ImGui::Checkbox("自动滚动", &app.log_auto_scroll);
    ImGui::SameLine();
    if (ImGui::Button("清空日志")) {
        std::lock_guard<std::mutex> lk(app.mu);
        app.logs.clear();
    }

    if (app.log_open) {
        std::vector<LogLine> lines;
        {
            std::lock_guard<std::mutex> lk(app.mu);
            lines = app.logs;
        }
        if (ImGui::BeginChild("##log_area", ImVec2(-1, 140), true)) {
            for (const auto& l : lines) {
                ImGui::TextDisabled("%s", l.time.c_str());
                ImGui::SameLine();
                if (l.level == "OK") {
                    ImGui::TextColored(ImVec4(0.09f, 0.64f, 0.29f, 1.0f), "[OK]");
                } else if (l.level == "WARN") {
                    ImGui::TextColored(ImVec4(0.85f, 0.47f, 0.02f, 1.0f), "[WARN]");
                } else if (l.level == "ERR") {
                    ImGui::TextColored(ImVec4(0.86f, 0.15f, 0.15f, 1.0f), "[ERR]");
                } else {
                    ImGui::TextDisabled("[INFO]");
                }
                ImGui::SameLine();
                ImGui::TextUnformatted(l.msg.c_str());
            }
            if (app.log_auto_scroll && ImGui::GetScrollY() < ImGui::GetScrollMaxY() - 1) {
                ImGui::SetScrollHereY(1.0f);
            }
        }
        ImGui::EndChild();
    }
}

}  // namespace

int run_gui(const Sections& sections,
            const std::vector<std::pair<std::string, std::string>>& opts) {
    if (!glfwInit()) {
        std::fprintf(stderr, "glfwInit failed\n");
        return 1;
    }
    glfwWindowHint(GLFW_CONTEXT_VERSION_MAJOR, 3);
    glfwWindowHint(GLFW_CONTEXT_VERSION_MINOR, 3);
    glfwWindowHint(GLFW_OPENGL_PROFILE, GLFW_OPENGL_CORE_PROFILE);
#ifdef __APPLE__
    glfwWindowHint(GLFW_OPENGL_FORWARD_COMPAT, GLFW_TRUE);
#endif

    GLFWwindow* window = glfwCreateWindow(920, 680, "sync-iobs", nullptr, nullptr);
    if (!window) {
        std::fprintf(stderr, "glfwCreateWindow failed\n");
        glfwTerminate();
        return 1;
    }
    glfwMakeContextCurrent(window);
    glfwSwapInterval(1);

    IMGUI_CHECKVERSION();
    ImGui::CreateContext();
    ImGuiIO& io = ImGui::GetIO();

    // 中文字体：Dear ImGui 内置字体不含中文，不挂系统字体全是方块
    io.Fonts->AddFontDefault();
    std::vector<std::pair<const char*, int>> candidates;
#ifdef _WIN32
    candidates.push_back({"C:\\Windows\\Fonts\\msyh.ttc", 0});
    candidates.push_back({"C:\\Windows\\Fonts\\simhei.ttf", 0});
#elif defined(__APPLE__)
    candidates.push_back({"/System/Library/Fonts/PingFang.ttc", 0});
    candidates.push_back({"/System/Library/Fonts/STHeiti Light.ttc", 0});
#else
    candidates.push_back({"/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc", 0});
    candidates.push_back({"/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc", 0});
#endif
    for (const auto& c : candidates) {
        ImFontConfig cfg;
        cfg.FontNo = c.second;   // .ttc 字体集合需指定字面序号
        cfg.MergeMode = true;
        cfg.OversampleH = 2;
        cfg.OversampleV = 2;
        if (io.Fonts->AddFontFromFileTTF(c.first, 16.0f, &cfg,
                                         io.Fonts->GetGlyphRangesChineseFull())) {
            std::printf("[font] %s\n", c.first);
            break;
        }
    }

    ImGui::StyleColorsLight();
    ImGuiStyle& style = ImGui::GetStyle();
    style.WindowRounding = 6.0f;
    style.FrameRounding = 4.0f;
    style.ItemSpacing = ImVec2(10, 10);
    style.FramePadding = ImVec2(10, 5);

    ImGui_ImplGlfw_InitForOpenGL(window, true);
    ImGui_ImplOpenGL3_Init("#version 330");

    App app;
    app.sections = sections;
    app.cfg = resolve_config(sections, opts);
    app.envs = env_names(sections);
    app.env = app.cfg.env;
    app.ak = app.cfg.access_key;
    app.sk = app.cfg.secret_key;
    for (const auto& kv : opts) {
        if (kv.first == "--config") app.cfg_path = kv.second;
    }
    if (app.cfg_path.empty()) {
        // 回退到自动查找
        app.cfg_path = "(自动查找)";
    }
    app.history_refresh.store(true);

    while (!glfwWindowShouldClose(window) && !app.quit) {
        glfwPollEvents();

        if (app.history_refresh.exchange(false)) refresh_history_async(app);

        ImGui_ImplOpenGL3_NewFrame();
        ImGui_ImplGlfw_NewFrame();
        ImGui::NewFrame();

        ImGui::SetNextWindowPos(ImVec2(0, 0));
        ImGui::SetNextWindowSize(io.DisplaySize);
        ImGui::Begin("sync-iobs", nullptr,
                     ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove |
                         ImGuiWindowFlags_NoResize |
                         ImGuiWindowFlags_NoBringToFrontOnFocus);

        float width = ImGui::GetContentRegionAvail().x;
        draw_header(app, width);
        ImGui::Separator();

        // 表单区：占满中部，可滚动
        ImGui::BeginChild("##forms", ImVec2(-1, -180), false);
        draw_forms(app);
        ImGui::EndChild();

        ImGui::Separator();
        draw_dock(app);

        ImGui::End();

        ImGui::Render();
        int w = 0, h = 0;
        glfwGetFramebufferSize(window, &w, &h);
        glViewport(0, 0, w, h);
        glClearColor(0.96f, 0.96f, 0.97f, 1.0f);
        glClear(GL_COLOR_BUFFER_BIT);
        ImGui_ImplOpenGL3_RenderDrawData(ImGui::GetDrawData());
        glfwSwapBuffers(window);
    }

    ImGui_ImplOpenGL3_Shutdown();
    ImGui_ImplGlfw_Shutdown();
    ImGui::DestroyContext();
    glfwDestroyWindow(window);
    glfwTerminate();
    return 0;
}

bool should_use_gui(int argc, char** argv) {
    if (argc <= 1) return true;
    const char* cmds[] = {"upload", "download", "history", "url", "init-config",
                          "--help", "-h", "--version"};
    std::string a = argv[1];
    for (const char* c : cmds) {
        if (a == c) return false;
    }
    return true;
}

}  // namespace siobs
