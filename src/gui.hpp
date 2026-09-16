#pragma once
// Dear ImGui 界面：三分区布局（顶栏 / 表单区 / 底部任务与日志）
// 与 Rust/egui 版保持同样的信息结构与交互。

#include "config.hpp"
#include "iobs.hpp"
#include "json.hpp"

#include <atomic>
#include <functional>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

namespace siobs {

/// 运行 GUI，返回进程退出码
int run_gui(const Sections& sections, const std::vector<std::pair<std::string, std::string>>& opts);

/// 供 main 判断是否需要走 GUI（无参数时默认 GUI，带子命令时走 CLI）
bool should_use_gui(int argc, char** argv);

}  // namespace siobs
