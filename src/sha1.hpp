#pragma once
// SHA-1 与 HMAC-SHA1 —— 手写实现，避免引入 OpenSSL 等依赖。
#include <array>
#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace siobs {

std::array<std::uint8_t, 20> sha1(const std::uint8_t* data, std::size_t len);
std::array<std::uint8_t, 20> sha1(const std::string& s);

std::vector<std::uint8_t> hmac_sha1(const std::uint8_t* key, std::size_t key_len,
                                    const std::uint8_t* msg, std::size_t msg_len);

/// 二进制转小写十六进制
std::string hex(const std::uint8_t* data, std::size_t len);

}  // namespace siobs
