#pragma once
// HTTP 客户端：Windows 用系统自带的 WinHTTP，macOS / Linux 用 libcurl。
// 这样 Windows 侧零第三方依赖（C++ 在 Windows 上配静态依赖最麻烦），
// Unix 侧则直接用系统自带的 curl。

#include <cstdint>
#include <functional>
#include <string>
#include <vector>

namespace siobs {

struct HttpResponse {
    long status = 0;
    std::string body;      // out_file 为空时，响应体读到这里
    std::string error;     // 传输层错误（连不上、超时等），空表示没出错
    bool ok() const { return status >= 200 && status < 300; }
};

/// 上传进度回调：done 已发送字节，total 总字节（未知时为 0）
using ProgressCb = std::function<void(std::uint64_t done, std::uint64_t total)>;

struct FormField {
    std::string name;
    std::string value;
};

struct FilePart {
    std::string field;         // 表单字段名
    std::string filename;      // 上传时用的文件名（可自定义后缀）
    std::string content_type;  // 为空则用 application/octet-stream
    const std::vector<std::uint8_t>* data = nullptr;
};

/// GET。out_file 为空则读进内存，否则边下边写文件。
HttpResponse http_get(const std::string& url, const std::string& out_file,
                      ProgressCb progress, bool insecure);

/// POST 原始字节（分片上传用）
HttpResponse http_post_raw(const std::string& url, const std::string& content_type,
                           const std::vector<std::uint8_t>& body, ProgressCb progress,
                           bool insecure);

/// POST multipart/form-data（小文件上传、completeUpload 用）
HttpResponse http_post_multipart(const std::string& url,
                                 const std::vector<FormField>& fields,
                                 const FilePart* file, bool insecure);

/// URL 编码（form 字段值、query 值）
std::string url_encode(const std::string& s);

}  // namespace siobs
