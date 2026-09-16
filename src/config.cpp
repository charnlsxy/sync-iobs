#include "config.hpp"

#include <algorithm>
#include <cstdio>
#include <fstream>
#include <sstream>

#if defined(_WIN32)
#include <windows.h>
#elif defined(__APPLE__)
#include <mach-o/dyld.h>
#else
#include <sys/types.h>
#include <unistd.h>
#endif

namespace siobs {

std::string trim(const std::string& s) {
    const char* ws = " \t\r\n";
    std::size_t b = s.find_first_not_of(ws);
    if (b == std::string::npos) return std::string();
    std::size_t e = s.find_last_not_of(ws);
    return s.substr(b, e - b + 1);
}

namespace {

/// 去掉行内注释：只处理前面有空白的 ; 或 #，避免误伤值本身
std::string strip_inline_comment(const std::string& line) {
    bool in_quote = false;
    for (std::size_t i = 0; i < line.size(); ++i) {
        char c = line[i];
        if (c == '"') in_quote = !in_quote;
        if (in_quote) continue;
        if (c == ';' || c == '#') {
            bool prev_is_space = (i == 0) || (line[i - 1] == ' ') || (line[i - 1] == '\t');
            if (prev_is_space) return line.substr(0, i);
        }
    }
    return line;
}

}  // namespace

Sections parse_ini(const std::string& text) {
    Sections out;
    std::string current = "common";
    out[current];  // 保证 common 段一定存在

    std::istringstream in(text);
    std::string raw;
    while (std::getline(in, raw)) {
        // 兼容 CRLF
        if (!raw.empty() && raw.back() == '\r') raw.pop_back();

        std::string line = trim(raw);
        if (line.empty()) continue;
        if (line[0] == ';' || line[0] == '#') continue;

        if (line[0] == '[') {
            std::size_t close = line.find(']');
            if (close == std::string::npos) continue;
            current = trim(line.substr(1, close - 1));
            if (current.empty()) current = "common";
            out[current];
            continue;
        }

        std::size_t eq = line.find('=');
        if (eq == std::string::npos) continue;
        std::string key = trim(line.substr(0, eq));
        std::string value = trim(strip_inline_comment(line.substr(eq + 1)));
        if (key.empty()) continue;
        out[current][key] = value;
    }
    return out;
}

std::vector<std::string> env_names(const Sections& secs) {
    std::vector<std::string> names;
    for (const auto& kv : secs) {
        if (kv.first != "common") names.push_back(kv.first);
    }
    std::sort(names.begin(), names.end());
    return names;
}

std::string find_config(const std::string& explicit_path) {
    if (!explicit_path.empty()) {
        std::ifstream f(explicit_path);
        if (f.good()) return explicit_path;
        return std::string();
    }

    std::vector<std::string> candidates;
    candidates.push_back("iobs.ini");

    // 程序所在目录下的 iobs.ini
    std::vector<char> buf(4096);
#if defined(_WIN32)
    DWORD len = GetModuleFileNameA(nullptr, buf.data(), static_cast<DWORD>(buf.size()));
    if (len > 0 && len < buf.size()) {
        std::string exe(buf.data(), len);
        std::size_t slash = exe.find_last_of("\\/");
        if (slash != std::string::npos) {
            candidates.push_back(exe.substr(0, slash) + "\\iobs.ini");
        }
    }
#elif defined(__APPLE__)
    uint32_t size = static_cast<uint32_t>(buf.size());
    if (_NSGetExecutablePath(buf.data(), &size) == 0) {
        std::string exe(buf.data());
        std::size_t slash = exe.find_last_of('/');
        if (slash != std::string::npos) {
            candidates.push_back(exe.substr(0, slash) + "/iobs.ini");
        }
    }
#else
    ssize_t len = readlink("/proc/self/exe", buf.data(), buf.size() - 1);
    if (len > 0) {
        buf[len] = '\0';
        std::string exe(buf.data());
        std::size_t slash = exe.find_last_of('/');
        if (slash != std::string::npos) {
            candidates.push_back(exe.substr(0, slash) + "/iobs.ini");
        }
    }
#endif

    for (const auto& p : candidates) {
        std::ifstream f(p);
        if (f.good()) return p;
    }
    return std::string();
}

}  // namespace siobs
