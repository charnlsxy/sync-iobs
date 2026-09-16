#include "base64.hpp"

namespace siobs {
namespace {

const char kEnc[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

int dec_value(char c) {
    if (c >= 'A' && c <= 'Z') return c - 'A';
    if (c >= 'a' && c <= 'z') return c - 'a' + 26;
    if (c >= '0' && c <= '9') return c - '0' + 52;
    if (c == '-' || c == '+') return 62;
    if (c == '_' || c == '/') return 63;
    return -1;
}

}  // namespace

std::string base64url_encode(const std::uint8_t* data, std::size_t len) {
    std::string out;
    out.reserve(((len + 2) / 3) * 4);
    std::size_t i = 0;
    while (i + 3 <= len) {
        std::uint32_t v = (std::uint32_t(data[i]) << 16) | (std::uint32_t(data[i + 1]) << 8) |
                          std::uint32_t(data[i + 2]);
        out.push_back(kEnc[(v >> 18) & 63]);
        out.push_back(kEnc[(v >> 12) & 63]);
        out.push_back(kEnc[(v >> 6) & 63]);
        out.push_back(kEnc[v & 63]);
        i += 3;
    }
    std::size_t rest = len - i;
    if (rest == 1) {
        std::uint32_t v = std::uint32_t(data[i]) << 16;
        out.push_back(kEnc[(v >> 18) & 63]);
        out.push_back(kEnc[(v >> 12) & 63]);
        out.push_back('=');
        out.push_back('=');
    } else if (rest == 2) {
        std::uint32_t v = (std::uint32_t(data[i]) << 16) | (std::uint32_t(data[i + 1]) << 8);
        out.push_back(kEnc[(v >> 18) & 63]);
        out.push_back(kEnc[(v >> 12) & 63]);
        out.push_back(kEnc[(v >> 6) & 63]);
        out.push_back('=');
    }
    return out;
}

std::string base64url_encode(const std::string& s) {
    return base64url_encode(reinterpret_cast<const std::uint8_t*>(s.data()), s.size());
}

std::vector<std::uint8_t> base64url_decode(const std::string& s) {
    std::vector<std::uint8_t> out;
    std::uint32_t acc = 0;
    int bits = 0;
    for (char c : s) {
        if (c == '=' || c == '\n' || c == '\r') continue;
        int v = dec_value(c);
        if (v < 0) continue;
        acc = (acc << 6) | std::uint32_t(v);
        bits += 6;
        if (bits >= 8) {
            bits -= 8;
            out.push_back(std::uint8_t((acc >> bits) & 0xff));
        }
    }
    return out;
}

}  // namespace siobs
