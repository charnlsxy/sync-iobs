#pragma once
#include <cstdint>
#include <string>
// std::string 必须显式包含：只依赖 <cstdint> 会在 MSVC 下报 C2530/C4430 一串连错

namespace siobs {

/// iobs 上传/下载凭证。
///
/// token = accessKey : sign : message
///   message = base64url( {"scope":"<bucket>:<key>","deadline":<秒级时间戳>} )   带 padding
///   sign    = base64url( HMAC-SHA1(key = SHA1(secretKey) 的 20 字节原始摘要,
///                                  msg = message 字符串) )
///
/// 注意 HMAC 的 key 是 SHA1(secretKey) 的**原始 20 字节**，不是十六进制文本。
std::string make_token(const std::string& access_key, const std::string& secret_key,
                       const std::string& bucket, const std::string& key,
                       std::uint64_t deadline_unix_secs);

}  // namespace siobs
