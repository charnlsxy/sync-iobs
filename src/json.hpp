#pragma once
// 极简 JSON：只实现本项目需要的部分
//   解析：[{"name":"..","key":"..","time":123}, ...]
//   生成：同上
// 不引入第三方库，保持依赖最小。

#include <cstdint>
#include <string>
#include <vector>

namespace siobs {

struct HistoryItem {
    std::string name;
    std::string key;
    std::uint64_t time = 0;
};

/// 解析历史记录数组；失败返回空 vector
std::vector<HistoryItem> parse_history(const std::string& text);

/// 生成历史记录 JSON（紧凑格式，与服务端既有数据保持一致）
std::string serialize_history(const std::vector<HistoryItem>& items);

/// 从 JSON 对象里取字符串字段值（用于解析 {"hash":"xxx"} 这类响应）
std::string json_get_string(const std::string& text, const std::string& field);

/// 生成 JSON 字符串字面量（带转义），不含外层引号
std::string json_escape(const std::string& s);

}  // namespace siobs
