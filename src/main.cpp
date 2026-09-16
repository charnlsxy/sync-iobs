// sync-iobs —— C++ / Dear ImGui 版入口
//
// 无参数启动 → 图形界面
// 带子命令     → 命令行模式（upload / download / history / url / init-config）

#include "config.hpp"
#include "gui.hpp"
#include "iobs.hpp"
#include "json.hpp"

#include <cstdio>
#include <fstream>
#include <iostream>
#include <sstream>
#include <string>
#include <vector>

namespace {

using namespace siobs;

void print_usage() {
    std::cout << R"(sync-iobs — iobs 文件双向同步

用法:
  sync-iobs                          启动图形界面
  sync-iobs upload <文件> [--key k] [--ext png]
  sync-iobs download <key> [--out 路径]
  sync-iobs history                  列出最近上传的 10 条记录
  sync-iobs url <key>                打印带 token 的下载地址
  sync-iobs init-config [路径]       生成一份配置模板

全局选项:
  -c, --config <文件>      指定配置文件
  --env <名称>             选择环境段
  --base-url <url>         覆盖 base_url
  --bucket <桶名>          覆盖 bucket
  --ak <key>               覆盖 access_key
  --sk <key>               覆盖 secret_key
  --insecure               跳过证书校验
)";
}

/// 简易参数解析：--key value 或 --flag
std::vector<std::pair<std::string, std::string>> parse_args(
    int argc, char** argv, std::vector<std::string>& positional) {
    std::vector<std::pair<std::string, std::string>> opts;
    for (int i = 1; i < argc; ++i) {
        std::string a = argv[i];
        if (a.rfind("--", 0) == 0 || (a.size() == 2 && a[0] == '-')) {
            if (i + 1 < argc && argv[i + 1][0] != '-') {
                opts.emplace_back(a, argv[i + 1]);
                ++i;
            } else {
                opts.emplace_back(a, "");
            }
        } else {
            positional.push_back(a);
        }
    }
    return opts;
}

const char* kTemplate =
    "# iobs 同步工具配置模板\n"
    "# 用 --env <名称> 选择下面的环境段，环境段内的配置优先于 [common]\n"
    "#\n"
    "# 本文件不含任何真实凭据与内网地址，所有字段都需要按实际环境填写\n"
    "\n"
    "[common]\n"
    "bucket           =            ; 必填：目标桶名\n"
    "access_key       =            ; 必填\n"
    "secret_key       =            ; 必填\n"
    "small_file_limit = 10MB       ; 小于该值走小文件直传\n"
    "chunk_size       = 5MB        ; 分片大小（服务端下限 5MB）\n"
    "timeout          = 60\n"
    "retry            = 3\n"
    "token_ttl        = 600\n"
    "\n"
    "[outer]\n"
    "base_url = https://iobs.example.com\n"
    "\n"
    "[inner]\n"
    "base_url = https://iobs-inner.example.com\n";

}  // namespace

int main(int argc, char** argv) {
    std::vector<std::string> positional;
    auto opts = parse_args(argc, argv, positional);

    for (const auto& kv : opts) {
        if (kv.first == "--help" || kv.first == "-h") {
            print_usage();
            return 0;
        }
    }

    // ---- init-config 不需要配置文件 ----
    if (!positional.empty() && positional[0] == "init-config") {
        std::string path = positional.size() > 1 ? positional[1] : "iobs.ini";
        std::ifstream probe(path);
        if (probe) {
            std::cerr << "文件已存在，未覆盖：" << path << "\n";
            return 1;
        }
        std::ofstream f(path, std::ios::binary);
        if (!f) {
            std::cerr << "无法写入：" << path << "\n";
            return 1;
        }
        f << kTemplate;
        std::cout << "已生成配置模板：" << path << "\n";
        return 0;
    }

    // ---- 读配置 ----
    std::string explicit_cfg;
    for (const auto& kv : opts) {
        if (kv.first == "--config" || kv.first == "-c") explicit_cfg = kv.second;
    }
    std::string cfg_path = find_config(explicit_cfg);
    Sections sections;
    if (!cfg_path.empty()) {
        std::ifstream f(cfg_path, std::ios::binary);
        std::ostringstream ss;
        ss << f.rdbuf();
        sections = parse_ini(ss.str());
    }

    // ---- 无子命令 → GUI ----
    if (siobs::should_use_gui(argc, argv)) {
        return siobs::run_gui(sections, opts);
    }

    Config cfg = resolve_config(sections, opts);
    const std::string& cmd = positional[0];

    auto need = [&]() -> bool {
        if (cfg.base_url.empty()) {
            std::cerr << "错误：缺少 base_url，请配置 iobs.ini 或使用 --base-url\n";
            return false;
        }
        if (cfg.bucket.empty()) {
            std::cerr << "错误：缺少 bucket，请配置 iobs.ini 或使用 --bucket\n";
            return false;
        }
        if (cfg.access_key.empty() || cfg.secret_key.empty()) {
            std::cerr << "错误：缺少 access_key / secret_key，请配置 iobs.ini 或使用 "
                         "--ak/--sk\n";
            return false;
        }
        return true;
    };

    if (cmd == "upload") {
        if (positional.size() < 2) {
            std::cerr << "用法：sync-iobs upload <文件>\n";
            return 1;
        }
        if (!need()) return 2;

        std::string path = positional[1];
        std::string key, ext;
        for (const auto& kv : opts) {
            if (kv.first == "--key") key = kv.second;
            if (kv.first == "--ext") ext = kv.second;
        }
        std::string name = path;
        std::size_t slash = name.find_last_of("/\\");
        if (slash != std::string::npos) name = name.substr(slash + 1);
        if (key.empty()) key = name;
        if (ext.empty()) ext = cfg.file_ext;
        if (!ext.empty()) {
            std::string stem = name;
            std::size_t dot = name.find_last_of('.');
            if (dot != std::string::npos) stem = name.substr(0, dot);
            std::string e = ext;
            while (!e.empty() && e.front() == '.') e.erase(e.begin());
            name = stem + "." + e;
        }

        std::string err = upload_file(cfg, path, key, name,
                                      [](std::uint64_t done, std::uint64_t total) {
                                          if (total == 0) return;
                                          int pct = static_cast<int>(done * 100 / total);
                                          std::printf("\r上传中 %3d%%", pct);
                                          std::fflush(stdout);
                                      });
        std::printf("\n");
        if (!err.empty()) {
            std::cerr << "上传失败：" << err << "\n";
            return 1;
        }
        std::cout << "上传完成：" << key << "\n";

        std::string rerr = record_upload(cfg, key, name);
        if (!rerr.empty()) std::cerr << "警告：" << rerr << "\n";
        return 0;
    }

    if (cmd == "download") {
        if (positional.size() < 2) {
            std::cerr << "用法：sync-iobs download <key>\n";
            return 1;
        }
        if (!need()) return 2;
        std::string key = positional[1];
        std::string out = key;
        for (const auto& kv : opts) {
            if (kv.first == "--out") out = kv.second;
        }
        if (!out.empty() && (out.back() == '/' || out.back() == '\\')) out += key;

        std::string err = download_to_file(cfg, key, out,
                                           [](std::uint64_t done, std::uint64_t) {
                                               std::printf("\r已接收 %llu 字节",
                                                           static_cast<unsigned long long>(done));
                                               std::fflush(stdout);
                                           });
        std::printf("\n");
        if (!err.empty()) {
            std::cerr << "下载失败：" << err << "\n";
            return 1;
        }
        std::cout << "已保存：" << out << "\n";
        return 0;
    }

    if (cmd == "history") {
        if (!need()) return 2;
        auto items = load_history(cfg);
        if (items.empty()) {
            std::cout << "暂无记录\n";
            return 0;
        }
        for (const auto& it : items) {
            std::cout << "  " << it.key << "  " << it.name << "\n";
        }
        return 0;
    }

    if (cmd == "url") {
        if (positional.size() < 2) {
            std::cerr << "用法：sync-iobs url <key>\n";
            return 1;
        }
        if (!need()) return 2;
        std::cout << download_url(cfg, positional[1]) << "\n";
        return 0;
    }

    std::cerr << "未知命令：" << cmd << "\n";
    print_usage();
    return 1;
}
