#include "token.hpp"

#include "base64.hpp"
#include "sha1.hpp"

#include <vector>

namespace siobs {

std::string make_token(const std::string& access_key, const std::string& secret_key,
                       const std::string& bucket, const std::string& key,
                       std::uint64_t deadline_unix_secs) {
    // 1) message：紧凑 JSON，字段顺序固定为 scope、deadline
    const std::string payload = "{\"scope\":\"" + bucket + ":" + key + "\",\"deadline\":" +
                                std::to_string(deadline_unix_secs) + "}";
    const std::string message = base64url_encode(payload);

    // 2) sign：HMAC-SHA1，key = SHA1(secretKey) 的原始 20 字节
    const auto sk_digest = sha1(secret_key);
    const auto sig = hmac_sha1(sk_digest.data(), sk_digest.size(),
                               reinterpret_cast<const std::uint8_t*>(message.data()),
                               message.size());
    const std::string sign = base64url_encode(sig.data(), sig.size());

    // 3) 拼接（冒号不做转义，原样拼进 query）
    return access_key + ":" + sign + ":" + message;
}

}  // namespace siobs
