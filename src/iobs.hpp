#pragma once
// iobs 协议实现：小文件直传、分片上传、下载、历史记录

#include "config.hpp"
#include "http.hpp"
#include "json.hpp"

#include <cstdint>
#include <functional>
#include <string>
#include <vector>

namespace siobs {

/// 历史记录存储的固定 key
extern const char* const kHistoryKey;

struct Config {
    std::string env = "outer";
    std::string base_url;
    std::string bucket;
    std::string access_key;
    std::string secret_key;
    std::uint64_t small_file_limit = 10ull * 1024 * 1024;
    std::uint64_t chunk_size = 5ull * 1024 * 1024;
    std::string file_ext;   // 为空则保持原后缀
    int timeout_secs = 60;
    int retry = 3;
    std::uint64_t token_ttl = 600;
    bool insecure = false;
};

/// 由 ini 段 + 命令行选项解析出最终配置。
/// opts 形如 {"--env":"outer", "--bucket":"xx"}，优先级高于配置文件。
Config resolve_config(const Sections& sections,
                      const std::vector<std::pair<std::string, std::string>>& opts);

/// 上传/下载进度：done 已完成字节，total 总字节（未知为 0）
using TransferProgress = std::function<void(std::uint64_t done, std::uint64_t total)>;

std::string upload_url(const Config& cfg, const std::string& key);
std::string download_url(const Config& cfg, const std::string& key);

/// 上传本地文件（自动选择小文件直传或分片）
/// 返回错误信息，空串表示成功
std::string upload_file(const Config& cfg, const std::string& path,
                        const std::string& key, const std::string& upload_name,
                        TransferProgress progress);

/// 下载到指定路径
std::string download_to_file(const Config& cfg, const std::string& key,
                             const std::string& out_path, TransferProgress progress);

/// 下载到内存
std::pair<std::string, std::string> download_to_memory(const Config& cfg,
                                                       const std::string& key);

/// 读取历史记录（key 不存在或解析失败都返回空）
std::vector<HistoryItem> load_history(const Config& cfg);

/// 记录一次上传：去重后置于最前，最多保留 10 条
std::string record_upload(const Config& cfg, const std::string& key,
                          const std::string& name);

}  // namespace siobs
