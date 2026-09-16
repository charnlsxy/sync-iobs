#include "sha1.hpp"

#include <cstring>

namespace siobs {
namespace {

inline std::uint32_t rotl(std::uint32_t v, int n) {
    return (v << n) | (v >> (32 - n));
}

struct Ctx {
    std::uint32_t h[5] = {0x67452301u, 0xEFCDAB89u, 0x98BADCFEu, 0x10325476u, 0xC3D2E1F0u};
    std::uint8_t buf[64] = {};
    std::size_t buflen = 0;
    std::uint64_t total = 0;

    void block(const std::uint8_t* p) {
        std::uint32_t w[80];
        for (int i = 0; i < 16; ++i) {
            w[i] = (std::uint32_t(p[i * 4]) << 24) | (std::uint32_t(p[i * 4 + 1]) << 16) |
                   (std::uint32_t(p[i * 4 + 2]) << 8) | std::uint32_t(p[i * 4 + 3]);
        }
        for (int i = 16; i < 80; ++i) {
            w[i] = rotl(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);
        }
        std::uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4];
        for (int i = 0; i < 80; ++i) {
            std::uint32_t f, k;
            if (i < 20) {
                f = (b & c) | ((~b) & d);
                k = 0x5A827999u;
            } else if (i < 40) {
                f = b ^ c ^ d;
                k = 0x6ED9EBA1u;
            } else if (i < 60) {
                f = (b & c) | (b & d) | (c & d);
                k = 0x8F1BBCDCu;
            } else {
                f = b ^ c ^ d;
                k = 0xCA62C1D6u;
            }
            std::uint32_t tmp = rotl(a, 5) + f + e + k + w[i];
            e = d;
            d = c;
            c = rotl(b, 30);
            b = a;
            a = tmp;
        }
        h[0] += a;
        h[1] += b;
        h[2] += c;
        h[3] += d;
        h[4] += e;
    }

    void update(const std::uint8_t* d, std::size_t n) {
        total += n;
        if (buflen) {
            std::size_t need = 64 - buflen;
            std::size_t take = n < need ? n : need;
            std::memcpy(buf + buflen, d, take);
            buflen += take;
            d += take;
            n -= take;
            if (buflen == 64) {
                block(buf);
                buflen = 0;
            }
        }
        while (n >= 64) {
            block(d);
            d += 64;
            n -= 64;
        }
        if (n) {
            std::memcpy(buf, d, n);
            buflen = n;
        }
    }

    std::array<std::uint8_t, 20> finish() {
        std::uint64_t bits = total * 8;
        const std::uint8_t one = 0x80;
        update(&one, 1);
        const std::uint8_t zero = 0;
        while (buflen != 56) update(&zero, 1);
        std::uint8_t len[8];
        for (int i = 0; i < 8; ++i) {
            len[i] = std::uint8_t((bits >> (56 - i * 8)) & 0xff);
        }
        update(len, 8);
        std::array<std::uint8_t, 20> out{};
        for (int i = 0; i < 5; ++i) {
            out[i * 4 + 0] = std::uint8_t((h[i] >> 24) & 0xff);
            out[i * 4 + 1] = std::uint8_t((h[i] >> 16) & 0xff);
            out[i * 4 + 2] = std::uint8_t((h[i] >> 8) & 0xff);
            out[i * 4 + 3] = std::uint8_t(h[i] & 0xff);
        }
        return out;
    }
};

}  // namespace

std::array<std::uint8_t, 20> sha1(const std::uint8_t* data, std::size_t len) {
    Ctx c;
    c.update(data, len);
    return c.finish();
}

std::array<std::uint8_t, 20> sha1(const std::string& s) {
    return sha1(reinterpret_cast<const std::uint8_t*>(s.data()), s.size());
}

std::vector<std::uint8_t> hmac_sha1(const std::uint8_t* key, std::size_t key_len,
                                    const std::uint8_t* msg, std::size_t msg_len) {
    std::vector<std::uint8_t> k(64, 0);
    if (key_len > 64) {
        auto h = sha1(key, key_len);
        std::memcpy(k.data(), h.data(), 20);
    } else {
        std::memcpy(k.data(), key, key_len);
    }
    std::vector<std::uint8_t> ipad(64), opad(64);
    for (int i = 0; i < 64; ++i) {
        ipad[i] = std::uint8_t(k[i] ^ 0x36);
        opad[i] = std::uint8_t(k[i] ^ 0x5c);
    }
    Ctx ci;
    ci.update(ipad.data(), 64);
    ci.update(msg, msg_len);
    auto hi = ci.finish();

    Ctx co;
    co.update(opad.data(), 64);
    co.update(hi.data(), 20);
    auto ho = co.finish();
    return std::vector<std::uint8_t>(ho.begin(), ho.end());
}

std::string hex(const std::uint8_t* data, std::size_t len) {
    static const char* digits = "0123456789abcdef";
    std::string out;
    out.reserve(len * 2);
    for (std::size_t i = 0; i < len; ++i) {
        out.push_back(digits[data[i] >> 4]);
        out.push_back(digits[data[i] & 0x0f]);
    }
    return out;
}

}  // namespace siobs
