// 不依赖任何 GUI / 网络的模块自测，可在纯命令行环境编译运行。
// 校验 SHA1、HMAC-SHA1、base64url 以及 iobs token 的构造是否与参考实现一致。

#include "../src/base64.hpp"
#include "../src/config.hpp"
#include "../src/sha1.hpp"
#include "../src/token.hpp"

#include <array>
#include <cstdio>
#include <string>

namespace {

int g_failed = 0;

void check(bool ok, const char* name, const std::string& got,
           const std::string& want) {
    if (ok) {
        std::printf("[ OK ] %s\n", name);
    } else {
        ++g_failed;
        std::printf("[FAIL] %s\n      got  = %s\n      want = %s\n", name,
                    got.c_str(), want.c_str());
    }
}

}  // namespace

int main() {
    using namespace siobs;

    // SHA1("abc") 标准向量
    {
        auto d = sha1(std::string("abc"));
        check(hex(d.data(), d.size()) ==
                  "a9993e364706816aba3e25717850c26c9cd0d89d",
              "sha1(abc)", hex(d.data(), d.size()),
              "a9993e364706816aba3e25717850c26c9cd0d89d");
    }

    // SHA1("") 标准向量
    {
        auto d = sha1(std::string(""));
        check(hex(d.data(), d.size()) ==
                  "da39a3ee5e6b4b0d3255bfef95601890afd80709",
              "sha1(empty)", hex(d.data(), d.size()),
              "da39a3ee5e6b4b0d3255bfef95601890afd80709");
    }

    // base64url：含 +/ 的字节必须转成 -_ 且保留 padding
    {
        const std::uint8_t raw[] = {0xfb, 0xff, 0xbf};
        std::string enc = base64url_encode(raw, sizeof(raw));
        check(enc == "-_-_", "base64url(0xfbffbf)", enc, "-_-_");
    }

    // token：与 Rust 版同一组测试向量（凭据为虚构值）
    {
        const std::string want =
            "TESTACCESSKEY000000000000:iaT60tYaaEAUHZfSgv5qEob40ww=:"
            "eyJzY29wZSI6InRlc3QtYnVja2V0OmYxNGQxZjU5LTc2N2ItNDA0ZS05M2IwLTgyNDU0Nzg2"
            "ZDk4MSIsImRlYWRsaW5lIjoxNzMyNzYxMTY1fQ==";
        std::string got = make_token("TESTACCESSKEY000000000000",
                                     "TESTSECRETKEY000000000000", "test-bucket",
                                     "f14d1f59-767b-404e-93b0-82454786d981", 1732761165);
        check(got == want, "make_token(vector)", got, want);
    }

    // INI 解析：行内注释、分段、环境名列表
    {
        const std::string ini =
            "# 顶部注释\n"
            "\n"
            "[common]\n"
            "bucket = test-bucket   ; 目标桶名\n"
            "access_key = AK # 行内注释\n"
            "\n"
            "[outer]\n"
            "base_url = https://a.example.com\n"
            "\n"
            "[inner]\n"
            "base_url = https://b.example.com\n";

        Sections secs = parse_ini(ini);
        check(secs["common"]["bucket"] == "test-bucket", "ini bucket",
              secs["common"]["bucket"], "test-bucket");
        check(secs["common"]["access_key"] == "AK", "ini inline comment stripped",
              secs["common"]["access_key"], "AK");
        check(secs["outer"]["base_url"] == "https://a.example.com", "ini outer url",
              secs["outer"]["base_url"], "https://a.example.com");

        auto names = env_names(secs);
        std::string joined;
        for (const auto& n : names) {
            if (!joined.empty()) joined += ",";
            joined += n;
        }
        check(joined == "inner,outer", "ini env names", joined, "inner,outer");
    }

    // 空 INI 也要有 common 段，且环境名为空
    {
        Sections secs = parse_ini("");
        check(secs.count("common") == 1, "empty ini has common",
              secs.count("common") ? "yes" : "no", "yes");
        check(env_names(secs).empty(), "empty ini has no env",
              std::to_string(env_names(secs).size()), "0");
    }

    if (g_failed == 0) {
        std::printf("\nall tests passed\n");
        return 0;
    }
    std::printf("\n%d test(s) failed\n", g_failed);
    return 1;
}
