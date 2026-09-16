#include "iobs.hpp"

#include "base64.hpp"
#include "sha1.hpp"
#include "token.hpp"

#include <algorithm>
#include <cstdio>
#include <cstdlib>
#include <ctime>
#include <fstream>

namespace siobs {

const char* const kHistoryKey = "cztesthistoryupload10";

namespace {

std::uint64_t now_secs() {
    return static_cast<std::uint64_t>(std::time(nullptr));
}

std::string read_file(const std::string& path, std::vector<std::uint8_t>& out) {
    std::ifstream f(path, std::ios::binary);
    if (!f) return "无法打开文件: " + path;
    f.seekg(0, std::ios::end);
    std::streamoff size = f.tellg();
    if (size < 0) return "无法获取文件大小: " + path;
    f.seekg(0, std::ios::beg);
    out.resize(static_cast<std::size_t>(size));
    if (size > 0) f.read(reinterpret_cast<char*>(out.data()), size);
    return std::string();
}

std::string find_opt(const std::vector<std::pair<std::string, std::string>>& opts,
                     const std::string& name) {
    for (const auto& kv : opts) {
        if (kv.first == name) return kv.second;
    }
    return std::string();
}

bool has_opt(const std::vector<std::pair<std::string, std::string>>& opts,
             const std::string& name) {
    for (const auto& kv : opts) {
        if (kv.first == name) return true;
    }
    return false;
}

std::string env_var(const char* name) {
    const char* v = std::getenv(name);
    return v ? std::string(v) : std::string();
}

std::uint64_t parse_size(const std::string& s) {
    // 支持 "10485760"、"10MB"、"5MB"、"512k" 这类写法
    std::string digits;
    std::string unit;
    for (char c : s) {
        if (std::isdigit(static_cast<unsigned char>(c)) || c == '.') {
            digits.push_back(c);
        } else if (!std::isspace(static_cast<unsigned char>(c))) {
            unit.push_back(static_cast<char>(std::tolower(static_cast<unsigned char>(c))));
        }
    }
    if (digits.empty()) return 0;
    double value = std::strtod(digits.c_str(), nullptr);
    double mul = 1.0;
    if (unit == "kb" || unit == "k") mul = 1024.0;
    else if (unit == "mb" || unit == "m") mul = 1024.0 * 1024.0;
    else if (unit == "gb" || unit == "g") mul = 1024.0 * 1024.0 * 1024.0;
    else if (unit == "b" || unit.empty()) mul = 1.0;
    return static_cast<std::uint64_t>(value * mul);
}

}  // namespace

Config resolve_config(const Sections& sections,
                      const std::vector<std::pair<std::string, std::string>>& opts) {
    Config cfg;

    auto common_it = sections.find("common");
    const Section common = (common_it != sections.end()) ? common_it->second : Section();

    std::string env = find_opt(opts, "--env");
    if (env.empty()) env = env_var("IOBS_ENV");
    if (env.empty()) env = "outer";
    cfg.env = env;

    auto env_it = sections.find(env);
    const Section env_sec = (env_it != sections.end()) ? env_it->second : Section();

    auto pick = [&](const std::string& key) -> std::string {
        auto a = env_sec.find(key);
        if (a != env_sec.end() && !a->second.empty()) return a->second;
        auto b = common.find(key);
        if (b != common.end()) return b->second;
        return std::string();
    };

    cfg.base_url = pick("base_url");
    cfg.bucket = pick("bucket");
    cfg.access_key = pick("access_key");
    cfg.secret_key = pick("secret_key");
    cfg.file_ext = pick("file_ext");

    std::string small = pick("small_file_limit");
    if (!small.empty()) cfg.small_file_limit = parse_size(small);
    std::string chunk = pick("chunk_size");
    if (!chunk.empty()) cfg.chunk_size = parse_size(chunk);
    std::string timeout = pick("timeout");
    if (!timeout.empty()) cfg.timeout_secs = std::atoi(timeout.c_str());
    std::string retry = pick("retry");
    if (!retry.empty()) cfg.retry = std::atoi(retry.c_str());
    std::string ttl = pick("token_ttl");
    if (!ttl.empty()) cfg.token_ttl = std::strtoull(ttl.c_str(), nullptr, 10);
    cfg.insecure = (pick("insecure") == "1" || pick("insecure") == "true");

    // 环境变量覆盖
    if (!env_var("IOBS_BASE_URL").empty()) cfg.base_url = env_var("IOBS_BASE_URL");
    if (!env_var("IOBS_BUCKET").empty()) cfg.bucket = env_var("IOBS_BUCKET");
    if (!env_var("IOBS_AK").empty()) cfg.access_key = env_var("IOBS_AK");
    if (!env_var("IOBS_SK").empty()) cfg.secret_key = env_var("IOBS_SK");

    // 命令行最高优先级
    if (!find_opt(opts, "--base-url").empty()) cfg.base_url = find_opt(opts, "--base-url");
    if (!find_opt(opts, "--bucket").empty()) cfg.bucket = find_opt(opts, "--bucket");
    if (!find_opt(opts, "--ak").empty()) cfg.access_key = find_opt(opts, "--ak");
    if (!find_opt(opts, "--sk").empty()) cfg.secret_key = find_opt(opts, "--sk");
    if (has_opt(opts, "--insecure")) cfg.insecure = true;

    while (!cfg.base_url.empty() && cfg.base_url.back() == '/') cfg.base_url.pop_back();

    // 服务端硬约束：分片小于 5MB 会被拒（EntityTooSmall）
    const std::uint64_t kMinChunk = 5ull * 1024 * 1024;
    if (cfg.chunk_size == 0 || cfg.chunk_size < kMinChunk) cfg.chunk_size = kMinChunk;
    if (cfg.small_file_limit == 0) cfg.small_file_limit = 10ull * 1024 * 1024;

    return cfg;
}

std::string upload_url(const Config& cfg, const std::string& key) {
    return cfg.base_url + "/upload/" + cfg.bucket + "/" + key;
}

std::string download_url(const Config& cfg, const std::string& key) {
    std::string token = make_token(cfg.access_key, cfg.secret_key, cfg.bucket, key,
                                   now_secs() + cfg.token_ttl);
    return cfg.base_url + "/download/" + cfg.bucket + "/" + key + "?token=" + token;
}

namespace {

/// 带重试的请求包装：仅对 5xx / 传输错误重试，4xx 直接返回
HttpResponse retry_request(const std::function<HttpResponse()>& fn, int retry) {
    HttpResponse last;
    for (int attempt = 0; attempt <= retry; ++attempt) {
        last = fn();
        if (!last.error.empty()) continue;                  // 传输错误 → 重试
        if (last.status >= 500 || last.status == 429) continue;  // 服务端错误 → 重试
        return last;
    }
    return last;
}

std::string describe(const HttpResponse& r) {
    if (!r.error.empty()) return r.error;
    std::string body = r.body;
    if (body.size() > 300) body = body.substr(0, 300) + "...";
    return "HTTP " + std::to_string(r.status) + " " + body;
}

}  // namespace

std::string upload_file(const Config& cfg, const std::string& path,
                        const std::string& key, const std::string& upload_name,
                        TransferProgress progress) {
    if (cfg.base_url.empty()) return "缺少 base_url，请先配置 iobs.ini";
    if (cfg.bucket.empty()) return "缺少 bucket，请先配置 iobs.ini";
    if (cfg.access_key.empty() || cfg.secret_key.empty()) {
        return "缺少 access_key / secret_key，请先配置 iobs.ini";
    }

    std::vector<std::uint8_t> data;
    std::string err = read_file(path, data);
    if (!err.empty()) return err;

    if (data.size() < cfg.small_file_limit) {
        // ---- 小文件直传 ----
        std::string token = make_token(cfg.access_key, cfg.secret_key, cfg.bucket, key,
                                       now_secs() + cfg.token_ttl);
        FilePart file{"file", upload_name, "", &data};
        std::vector<FormField> fields{{"token", token}};

        HttpResponse r = retry_request(
            [&] { return http_post_multipart(upload_url(cfg, key), fields, &file,
                                             cfg.insecure); },
            cfg.retry);
        if (progress) progress(data.size(), data.size());
        if (!r.ok()) return "上传失败：" + describe(r);
        return std::string();
    }

    // ---- 分片上传 ----
    const std::uint64_t chunk = cfg.chunk_size;
    const std::uint64_t total = data.size();
    const std::string init_token = make_token(cfg.access_key, cfg.secret_key,
                                              cfg.bucket, key, now_secs() + cfg.token_ttl);
    std::string init_url = cfg.base_url + "/initUploadPart/" + cfg.bucket + "/" + key +
                           "?token=" + url_encode(init_token) +
                           "&fileSize=" + std::to_string(total) +
                           "&fileName=" + url_encode(upload_name);

    HttpResponse init = retry_request(
        [&] { return http_post_raw(init_url, "application/x-www-form-urlencoded",
                                   std::vector<std::uint8_t>(), nullptr, cfg.insecure); },
        cfg.retry);
    if (!init.ok()) return "初始化分片上传失败：" + describe(init);

    std::string upload_id = trim(init.body);
    if (upload_id.empty()) return "初始化分片上传失败：服务端未返回 uploadId";

    // 上传各分片，收集服务端返回的 hash
    std::vector<std::string> hashes;
    std::uint64_t offset = 0;
    int part_number = 1;
    while (offset < total) {
        std::uint64_t len = std::min(chunk, total - offset);
        std::vector<std::uint8_t> piece(data.begin() + static_cast<long>(offset),
                                        data.begin() + static_cast<long>(offset + len));

        std::string ptoken = make_token(cfg.access_key, cfg.secret_key, cfg.bucket, key,
                                        now_secs() + cfg.token_ttl);
        std::string purl = cfg.base_url + "/uploadPart/" + cfg.bucket + "/" + key +
                           "?token=" + url_encode(ptoken) +
                           "&uploadId=" + url_encode(upload_id) +
                           "&partNumber=" + std::to_string(part_number);

        HttpResponse pr = retry_request(
            [&] {
                return http_post_raw(purl, "application/octet-stream", piece,
                                     nullptr, cfg.insecure);
            },
            cfg.retry);
        if (!pr.ok()) return "第 " + std::to_string(part_number) + " 个分片上传失败：" +
                              describe(pr);

        std::string hash = json_get_string(pr.body, "hash");
        if (hash.empty()) hash = trim(pr.body);
        hashes.push_back(hash);

        offset += len;
        ++part_number;
        if (progress) progress(offset, total);
    }

    // 完成：partMap 是 {"1":"hash", ...} 的 JSON，作为 form 字段提交
    std::string part_map = "{";
    for (std::size_t i = 0; i < hashes.size(); ++i) {
        if (i) part_map += ",";
        part_map += "\"" + std::to_string(i + 1) + "\":\"" + json_escape(hashes[i]) + "\"";
    }
    part_map += "}";

    std::string ctoken = make_token(cfg.access_key, cfg.secret_key, cfg.bucket, key,
                                    now_secs() + cfg.token_ttl);
    std::string curl = cfg.base_url + "/completeUpload/" + cfg.bucket + "/" + key +
                       "?token=" + url_encode(ctoken) +
                       "&uploadId=" + url_encode(upload_id);

    std::vector<FormField> cfields{{"partMap", part_map}};
    HttpResponse cr = retry_request(
        [&] { return http_post_multipart(curl, cfields, nullptr, cfg.insecure); },
        cfg.retry);
    if (!cr.ok()) return "合并分片失败：" + describe(cr);
    return std::string();
}

std::string download_to_file(const Config& cfg, const std::string& key,
                             const std::string& out_path, TransferProgress progress) {
    if (cfg.base_url.empty() || cfg.bucket.empty()) return "请先配置 base_url 与 bucket";
    if (cfg.access_key.empty() || cfg.secret_key.empty()) {
        return "缺少 access_key / secret_key，请先配置 iobs.ini";
    }
    HttpResponse r = retry_request(
        [&] { return http_get(download_url(cfg, key), out_path, progress, cfg.insecure); },
        cfg.retry);
    if (!r.ok()) return "下载失败：" + describe(r);
    return std::string();
}

std::pair<std::string, std::string> download_to_memory(const Config& cfg,
                                                       const std::string& key) {
    if (cfg.base_url.empty() || cfg.bucket.empty()) {
        return {"请先配置 base_url 与 bucket", ""};
    }
    HttpResponse r = retry_request(
        [&] {
            return http_get(download_url(cfg, key), std::string(), nullptr, cfg.insecure);
        },
        cfg.retry);
    if (!r.ok()) return {describe(r), ""};
    return {"", r.body};
}

std::vector<HistoryItem> load_history(const Config& cfg) {
    if (cfg.base_url.empty() || cfg.bucket.empty()) return {};
    if (cfg.access_key.empty() || cfg.secret_key.empty()) return {};

    HttpResponse r = http_get(download_url(cfg, kHistoryKey), std::string(), nullptr,
                              cfg.insecure);
    if (!r.ok()) return {};
    return parse_history(r.body);
}

std::string record_upload(const Config& cfg, const std::string& key,
                          const std::string& name) {
    std::vector<HistoryItem> items = load_history(cfg);

    // 同一 key 只保留最新一条：先移除旧的，再插到最前
    items.erase(std::remove_if(items.begin(), items.end(),
                               [&](const HistoryItem& it) { return it.key == key; }),
                items.end());
    HistoryItem item;
    item.name = name;
    item.key = key;
    item.time = now_secs();
    items.insert(items.begin(), item);
    if (items.size() > 10) items.resize(10);

    // 以小文件上传的方式写回；服务端会拒绝部分后缀，这里统一用 .png
    const std::string payload = serialize_history(items);
    std::vector<std::uint8_t> data(payload.begin(), payload.end());
    const std::string store_name = std::string(kHistoryKey) + ".png";

    std::string token = make_token(cfg.access_key, cfg.secret_key, cfg.bucket,
                                   kHistoryKey, now_secs() + cfg.token_ttl);
    FilePart file{"file", store_name, "", &data};
    std::vector<FormField> fields{{"token", token}};

    HttpResponse r = retry_request(
        [&] {
            return http_post_multipart(upload_url(cfg, kHistoryKey), fields, &file,
                                       cfg.insecure);
        },
        cfg.retry);
    if (!r.ok()) return "写入历史记录失败：" + describe(r);
    return std::string();
}

}  // namespace siobs
