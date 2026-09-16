#pragma once
#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace siobs {

/// URL 安全 base64（用 -_ 替代 +/），保留 '=' 填充 —— iobs 的 token 用的就是这种
std::string base64url_encode(const std::uint8_t* data, std::size_t len);
std::string base64url_encode(const std::string& s);

std::vector<std::uint8_t> base64url_decode(const std::string& s);

}  // namespace siobs
