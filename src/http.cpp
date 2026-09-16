#include "http.hpp"

#include <cstdio>
#include <cstring>
#include <sstream>

#ifdef _WIN32

#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN
#endif
#include <windows.h>
#include <winhttp.h>

#else

#include <curl/curl.h>

#endif

namespace siobs {

std::string url_encode(const std::string& s) {
    static const char* hex = "0123456789ABCDEF";
    std::string out;
    out.reserve(s.size() * 3);
    for (unsigned char c : s) {
        // 只保留无歧义字符；其余全部转义
        bool keep = (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') ||
                    (c >= '0' && c <= '9') || c == '-' || c == '_' || c == '.' || c == '~';
        if (keep) {
            out.push_back(static_cast<char>(c));
        } else {
            out.push_back('%');
            out.push_back(hex[c >> 4]);
            out.push_back(hex[c & 0x0f]);
        }
    }
    return out;
}

namespace {

std::vector<std::uint8_t> build_multipart(const std::vector<FormField>& fields,
                                          const FilePart* file,
                                          std::string& out_boundary) {
    const std::string boundary = "----siobsBoundary" + std::to_string(
        reinterpret_cast<std::uintptr_t>(&out_boundary));
    out_boundary = boundary;

    std::string body;
    auto append = [&body](const std::string& s) {
        body.append(s.data(), s.size());
    };

    for (const auto& f : fields) {
        append("--" + boundary + "\r\n");
        append("Content-Disposition: form-data; name=\"" + f.name + "\"\r\n\r\n");
        append(f.value + "\r\n");
    }
    if (file && file->data) {
        append("--" + boundary + "\r\n");
        append("Content-Disposition: form-data; name=\"" + file->field +
               "\"; filename=\"" + file->filename + "\"\r\n");
        append("Content-Type: " +
               (file->content_type.empty() ? std::string("application/octet-stream")
                                           : file->content_type) +
               "\r\n\r\n");
        body.append(reinterpret_cast<const char*>(file->data->data()), file->data->size());
        append("\r\n");
    }
    append("--" + boundary + "--\r\n");

    return std::vector<std::uint8_t>(body.begin(), body.end());
}

}  // namespace

#ifdef _WIN32
// ---------------------------------------------------------------- WinHTTP --

namespace {

std::wstring utf8_to_wide(const std::string& s) {
    if (s.empty()) return std::wstring();
    int n = MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()),
                                nullptr, 0);
    std::wstring out(static_cast<std::size_t>(n), L'\0');
    MultiByteToWideChar(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), &out[0], n);
    return out;
}

std::string wide_to_utf8(const std::wstring& s) {
    if (s.empty()) return std::string();
    int n = WideCharToMultiByte(CP_UTF8, 0, s.data(), static_cast<int>(s.size()),
                                nullptr, 0, nullptr, nullptr);
    std::string out(static_cast<std::size_t>(n), '\0');
    WideCharToMultiByte(CP_UTF8, 0, s.data(), static_cast<int>(s.size()), &out[0], n,
                        nullptr, nullptr);
    return out;
}

struct Cracked {
    bool https = false;
    std::wstring host;
    int port = 0;
    std::wstring path;
};

bool crack_url(const std::string& url, Cracked& out) {
    std::wstring w = utf8_to_wide(url);
    URL_COMPONENTS comp{};
    comp.dwStructSize = sizeof(comp);
    wchar_t host[256] = {}, path[4096] = {};
    comp.lpszHostName = host;
    comp.dwHostNameLength = 256;
    comp.lpszUrlPath = path;
    comp.dwUrlPathLength = 4096;
    if (!WinHttpCrackUrl(w.c_str(), static_cast<DWORD>(w.size()), 0, &comp)) {
        return false;
    }
    out.https = (comp.nScheme == INTERNET_SCHEME_HTTPS);
    out.host = host;
    out.port = comp.nPort;
    out.path = path;
    return true;
}

/// 统一的请求实现：返回响应体（写入 out_file 时 body 为空）
HttpResponse win_request(const std::wstring& verb, const std::string& url,
                         const std::string& content_type,
                         const std::vector<std::uint8_t>* body,
                         const std::string& out_file, ProgressCb progress,
                         bool insecure) {
    HttpResponse resp;
    Cracked u;
    if (!crack_url(url, u)) {
        resp.error = "无法解析 URL: " + url;
        return resp;
    }

    HINTERNET session = WinHttpOpen(L"sync-iobs/1.0", WINHTTP_ACCESS_TYPE_NO_PROXY,
                                    WINHTTP_NO_PROXY_NAME, WINHTTP_NO_PROXY_BYPASS, 0);
    if (!session) {
        resp.error = "WinHttpOpen 失败";
        return resp;
    }
    // 超时：连接 10s / 发送 30s / 接收 30s
    WinHttpSetTimeouts(session, 10000, 30000, 30000, 30000);

    HINTERNET conn = WinHttpConnect(session, u.host.c_str(),
                                    static_cast<INTERNET_PORT>(u.port), 0);
    if (!conn) {
        resp.error = "WinHttpConnect 失败";
        WinHttpCloseHandle(session);
        return resp;
    }

    HINTERNET req = WinHttpOpenRequest(
        conn, verb.c_str(), u.path.c_str(), nullptr, WINHTTP_NO_REFERER,
        WINHTTP_DEFAULT_ACCEPT_TYPES, u.https ? WINHTTP_FLAG_SECURE : 0);
    if (!req) {
        resp.error = "WinHttpOpenRequest 失败";
        WinHttpCloseHandle(conn);
        WinHttpCloseHandle(session);
        return resp;
    }

    if (u.https && insecure) {
        DWORD flags = SECURITY_FLAG_IGNORE_CERT_CN_INVALID |
                      SECURITY_FLAG_IGNORE_CERT_DATE_INVALID |
                      SECURITY_FLAG_IGNORE_UNKNOWN_CA;
        WinHttpSetOption(req, WINHTTP_OPTION_SECURITY_FLAGS, &flags, sizeof(flags));
    }

    std::wstring headers;
    if (!content_type.empty()) {
        headers = L"Content-Type: " + utf8_to_wide(content_type) + L"\r\n";
    }

    DWORD total = body ? static_cast<DWORD>(body->size()) : 0;
    BOOL sent = WinHttpSendRequest(
        req, headers.empty() ? WINHTTP_NO_REQUEST_DATA : headers.c_str(),
        headers.empty() ? 0 : static_cast<DWORD>(headers.size()),
        body && !body->empty() ? const_cast<std::uint8_t*>(body->data()) : WINHTTP_NO_REQUEST_DATA,
        body ? total : 0, body ? total : 0, 0);

    if (!sent) {
        resp.error = "WinHttpSendRequest 失败 (GetLastError=" +
                     std::to_string(GetLastError()) + ")";
        WinHttpCloseHandle(req);
        WinHttpCloseHandle(conn);
        WinHttpCloseHandle(session);
        return resp;
    }

    if (!WinHttpReceiveResponse(req, nullptr)) {
        resp.error = "WinHttpReceiveResponse 失败";
        WinHttpCloseHandle(req);
        WinHttpCloseHandle(conn);
        WinHttpCloseHandle(session);
        return resp;
    }

    DWORD status = 0;
    DWORD status_size = sizeof(status);
    if (WinHttpQueryHeaders(req, WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                            WINHTTP_HEADER_NAME_BY_INDEX, &status, &status_size,
                            WINHTTP_NO_HEADER_INDEX)) {
        resp.status = static_cast<long>(status);
    }

    FILE* fp = nullptr;
    if (!out_file.empty()) {
        fp = std::fopen(out_file.c_str(), "wb");
        if (!fp) {
            resp.error = "无法写入文件: " + out_file;
            WinHttpCloseHandle(req);
            WinHttpCloseHandle(conn);
            WinHttpCloseHandle(session);
            return resp;
        }
    }

    std::uint64_t got = 0;
    DWORD available = 0;
    while (WinHttpQueryDataAvailable(req, &available) && available > 0) {
        std::vector<char> chunk(available);
        DWORD read = 0;
        if (!WinHttpReadData(req, chunk.data(), available, &read) || read == 0) break;
        got += read;
        if (fp) {
            std::fwrite(chunk.data(), 1, read, fp);
        } else {
            resp.body.append(chunk.data(), read);
        }
        if (progress) progress(got, 0);
    }
    if (fp) std::fclose(fp);

    WinHttpCloseHandle(req);
    WinHttpCloseHandle(conn);
    WinHttpCloseHandle(session);
    return resp;
}

}  // namespace

HttpResponse http_get(const std::string& url, const std::string& out_file,
                      ProgressCb progress, bool insecure) {
    return win_request(L"GET", url, std::string(), nullptr, out_file, progress,
                       insecure);
}

HttpResponse http_post_raw(const std::string& url, const std::string& content_type,
                           const std::vector<std::uint8_t>& body, ProgressCb progress,
                           bool insecure) {
    return win_request(L"POST", url, content_type, &body, std::string(), progress,
                       insecure);
}

HttpResponse http_post_multipart(const std::string& url,
                                 const std::vector<FormField>& fields,
                                 const FilePart* file, bool insecure) {
    std::string boundary;
    std::vector<std::uint8_t> body = build_multipart(fields, file, boundary);
    return win_request(L"POST", url, "multipart/form-data; boundary=" + boundary,
                       &body, std::string(), nullptr, insecure);
}

#else
// ----------------------------------------------------------------- libcurl --

namespace {

std::size_t write_memory(void* contents, std::size_t size, std::size_t nmemb,
                         void* userp) {
    std::size_t total = size * nmemb;
    auto* s = static_cast<std::string*>(userp);
    s->append(static_cast<char*>(contents), total);
    return total;
}

struct FileSink {
    FILE* fp = nullptr;
    ProgressCb progress;
    std::uint64_t got = 0;
};

std::size_t write_file(void* contents, std::size_t size, std::size_t nmemb,
                       void* userp) {
    std::size_t total = size * nmemb;
    auto* sink = static_cast<FileSink*>(userp);
    std::fwrite(contents, 1, total, sink->fp);
    sink->got += total;
    if (sink->progress) sink->progress(sink->got, 0);
    return total;
}

int progress_cb(void* clientp, curl_off_t dltotal, curl_off_t dlnow,
                curl_off_t ultotal, curl_off_t ulnow) {
    auto* cb = static_cast<ProgressCb*>(clientp);
    if (!cb || !(*cb)) return 0;
    if (ultotal > 0) {
        (*cb)(static_cast<std::uint64_t>(ulnow), static_cast<std::uint64_t>(ultotal));
    } else if (dltotal > 0) {
        (*cb)(static_cast<std::uint64_t>(dlnow), static_cast<std::uint64_t>(dltotal));
    }
    return 0;
}

CURL* make_curl(const std::string& url, bool insecure, ProgressCb* progress,
                HttpResponse& resp) {
    CURL* c = curl_easy_init();
    if (!c) {
        resp.error = "curl_easy_init 失败";
        return nullptr;
    }
    curl_easy_setopt(c, CURLOPT_URL, url.c_str());
    curl_easy_setopt(c, CURLOPT_FOLLOWLOCATION, 1L);
    curl_easy_setopt(c, CURLOPT_CONNECTTIMEOUT_MS, 10000L);
    curl_easy_setopt(c, CURLOPT_TIMEOUT_MS, 0L);  // 大文件不设总超时
    if (insecure) {
        curl_easy_setopt(c, CURLOPT_SSL_VERIFYPEER, 0L);
        curl_easy_setopt(c, CURLOPT_SSL_VERIFYHOST, 0L);
    }
    if (progress) {
        curl_easy_setopt(c, CURLOPT_XFERINFOFUNCTION, progress_cb);
        curl_easy_setopt(c, CURLOPT_XFERINFODATA, progress);
        curl_easy_setopt(c, CURLOPT_NOPROGRESS, 0L);
    }
    return c;
}

void finish_curl(CURL* c, HttpResponse& resp) {
    long status = 0;
    curl_easy_getinfo(c, CURLINFO_RESPONSE_CODE, &status);
    resp.status = status;
    curl_easy_cleanup(c);
}

}  // namespace

HttpResponse http_get(const std::string& url, const std::string& out_file,
                      ProgressCb progress, bool insecure) {
    HttpResponse resp;
    CURL* c = make_curl(url, insecure, &progress, resp);
    if (!c) return resp;

    if (out_file.empty()) {
        curl_easy_setopt(c, CURLOPT_WRITEFUNCTION, write_memory);
        curl_easy_setopt(c, CURLOPT_WRITEDATA, &resp.body);
        CURLcode rc = curl_easy_perform(c);
        if (rc != CURLE_OK) resp.error = curl_easy_strerror(rc);
    } else {
        FILE* fp = std::fopen(out_file.c_str(), "wb");
        if (!fp) {
            resp.error = "无法写入文件: " + out_file;
            curl_easy_cleanup(c);
            return resp;
        }
        FileSink sink{fp, progress, 0};
        curl_easy_setopt(c, CURLOPT_WRITEFUNCTION, write_file);
        curl_easy_setopt(c, CURLOPT_WRITEDATA, &sink);
        CURLcode rc = curl_easy_perform(c);
        std::fclose(fp);
        if (rc != CURLE_OK) resp.error = curl_easy_strerror(rc);
    }
    finish_curl(c, resp);
    return resp;
}

HttpResponse http_post_raw(const std::string& url, const std::string& content_type,
                           const std::vector<std::uint8_t>& body, ProgressCb progress,
                           bool insecure) {
    HttpResponse resp;
    CURL* c = make_curl(url, insecure, &progress, resp);
    if (!c) return resp;

    struct curl_slist* headers = nullptr;
    std::string ct = "Content-Type: " + content_type;
    headers = curl_slist_append(headers, ct.c_str());
    curl_easy_setopt(c, CURLOPT_HTTPHEADER, headers);
    curl_easy_setopt(c, CURLOPT_POST, 1L);
    curl_easy_setopt(c, CURLOPT_POSTFIELDS, body.data());
    curl_easy_setopt(c, CURLOPT_POSTFIELDSIZE, static_cast<long>(body.size()));

    curl_easy_setopt(c, CURLOPT_WRITEFUNCTION, write_memory);
    curl_easy_setopt(c, CURLOPT_WRITEDATA, &resp.body);

    CURLcode rc = curl_easy_perform(c);
    if (rc != CURLE_OK) resp.error = curl_easy_strerror(rc);
    curl_slist_free_all(headers);
    finish_curl(c, resp);
    return resp;
}

HttpResponse http_post_multipart(const std::string& url,
                                 const std::vector<FormField>& fields,
                                 const FilePart* file, bool insecure) {
    HttpResponse resp;
    ProgressCb no_progress;
    CURL* c = make_curl(url, insecure, &no_progress, resp);
    if (!c) return resp;

    curl_mime* mime = curl_mime_init(c);
    for (const auto& f : fields) {
        curl_mimepart* part = curl_mime_addpart(mime);
        curl_mime_name(part, f.name.c_str());
        curl_mime_data(part, f.value.c_str(), CURL_ZERO_TERMINATED);
    }
    if (file && file->data) {
        curl_mimepart* part = curl_mime_addpart(mime);
        curl_mime_name(part, file->field.c_str());
        curl_mime_filename(part, file->filename.c_str());
        curl_mime_type(part, file->content_type.empty() ? "application/octet-stream"
                                                        : file->content_type.c_str());
        curl_mime_data(part, reinterpret_cast<const char*>(file->data->data()),
                       file->data->size());
    }
    curl_easy_setopt(c, CURLOPT_MIMEPOST, mime);

    curl_easy_setopt(c, CURLOPT_WRITEFUNCTION, write_memory);
    curl_easy_setopt(c, CURLOPT_WRITEDATA, &resp.body);

    CURLcode rc = curl_easy_perform(c);
    if (rc != CURLE_OK) resp.error = curl_easy_strerror(rc);
    finish_curl(c, resp);
    curl_mime_free(mime);
    return resp;
}

#endif

}  // namespace siobs
