#pragma once
// iobs.ini 解析：与 Rust 版保持同样语义
//   - [section] 分段，首个 section 之前的内容归入 common
//   - ; 或 # 开头的行为注释
//   - 值后面的行内注释（前面有空白的 ; 或 #）会被去掉
#include <map>
#include <string>
#include <vector>

namespace siobs {

using Section = std::map<std::string, std::string>;
using Sections = std::map<std::string, Section>;

/// 去掉首尾空白
std::string trim(const std::string& s);

/// 解析 INI 文本
Sections parse_ini(const std::string& text);

/// 环境段名列表（排除 common），已排序
std::vector<std::string> env_names(const Sections& secs);

/// 查找配置文件的路径顺序（与 Rust 版一致）：
///   显式指定 > 当前目录 > 程序所在目录
std::string find_config(const std::string& explicit_path);

}  // namespace siobs
