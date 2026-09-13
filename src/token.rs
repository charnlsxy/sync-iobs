// 自实现 SHA1 / HMAC-SHA1 / base64url，零外部依赖
// 用于生成 iobs 的上传 token： ak:signStr:messageStr

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let bit_len = (data.len() as u64).wrapping_mul(8);

    let mut msg: Vec<u8> = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for chunk in msg.chunks(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[i * 4], chunk[i * 4 + 1], chunk[i * 4 + 2], chunk[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for i in 0..80 {
            let (f, k) = match i / 20 {
                0 => ((b & c) | ((!b) & d), 0x5A827999u32),
                1 => (b ^ c ^ d, 0x6ED9EBA1u32),
                2 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDCu32),
                _ => (b ^ c ^ d, 0xCA62C1D6u32),
            };
            let tmp = a.rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = tmp;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }

    let mut out = [0u8; 20];
    for i in 0..5 {
        out[i * 4..i * 4 + 4].copy_from_slice(&h[i].to_be_bytes());
    }
    out
}

pub fn hmac_sha1(key: &[u8], msg: &[u8]) -> [u8; 20] {
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    let k: Vec<u8> = if key.len() > 64 { sha1(key).to_vec() } else { key.to_vec() };
    for i in 0..64 {
        let b = if i < k.len() { k[i] } else { 0 };
        ipad[i] ^= b;
        opad[i] ^= b;
    }
    let mut inner = Vec::with_capacity(64 + msg.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(msg);
    let ih = sha1(&inner);

    let mut outer = Vec::with_capacity(84);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&ih);
    sha1(&outer)
}

const B64: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// url-safe base64，**带 padding**（与 Java Base64.getUrlEncoder() 行为一致）
pub fn b64url(data: &[u8]) -> String {
    let mut s = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = ((c[0] as u32) << 16)
            | ((*c.get(1).unwrap_or(&0) as u32) << 8)
            | (*c.get(2).unwrap_or(&0) as u32);
        s.push(B64[((n >> 18) & 63) as usize] as char);
        s.push(B64[((n >> 12) & 63) as usize] as char);
        s.push(if c.len() > 1 { B64[((n >> 6) & 63) as usize] as char } else { '=' });
        s.push(if c.len() > 2 { B64[(n & 63) as usize] as char } else { '=' });
    }
    s
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub struct Signer {
    ak: String,
    sk_hash: [u8; 20],
}

impl Signer {
    pub fn new(ak: &str, secret_key: &str) -> Signer {
        Signer {
            ak: ak.to_string(),
            sk_hash: sha1(secret_key.as_bytes()),
        }
    }

    /// ttl: 有效期（秒），deadline = now + ttl
    pub fn token_at(&self, bucket: &str, key: &str, deadline: u64) -> String {
        let json = format!("{{\"scope\":\"{}:{}\",\"deadline\":{}}}", bucket, key, deadline);
        let message = b64url(json.as_bytes());
        let sign = b64url(&hmac_sha1(&self.sk_hash, message.as_bytes()));
        format!("{}:{}:{}", self.ak, sign, message)
    }

    pub fn token(&self, bucket: &str, key: &str, ttl: u64) -> String {
        self.token_at(bucket, key, now_secs() + ttl)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha1_vectors() {
        assert_eq!(hex(&sha1(b"")), "da39a3ee5e6b4b0d3255bfef95601890afd80709");
        assert_eq!(hex(&sha1(b"abc")), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(&sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq")),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1"
        );
        // 长输入（跨多个 512 位块 + padding 边界）
        let long = "a".repeat(1000);
        assert_eq!(hex(&sha1(long.as_bytes())), "291e9a6c66994949b57ba5e650361e98fc36b1ba");
    }

    #[test]
    fn hmac_rfc2202() {
        // RFC 2202 test case 1
        let k = vec![0x0bu8; 20];
        assert_eq!(hex(&hmac_sha1(&k, b"Hi There")), "b617318655057264e28bc0b6fb378c8ef146be00");
        // RFC 2202 test case 3
        assert_eq!(
            hex(&hmac_sha1(&vec![0xaau8; 20], &vec![0xddu8; 50])),
            "125d7342b9ac11cd91a39af48aa17b4f63f175d3"
        );
        // RFC 2202 test case 6：key 长于 64 字节，走 sha1 压缩分支
        assert_eq!(
            hex(&hmac_sha1(
                &vec![0xaau8; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112"
        );
    }

    #[test]
    fn matches_reference_token() {
        // 参考向量：签名算法与内网 Java 实现逐字节一致（SHA1(sk) 为 HMAC key，
        // 对 base64url(scope JSON) 做 HMAC-SHA1，拼成 ak:sign:msg）。
        // 注意：此处使用**虚构凭据**，仅用于校验签名算法的正确性。
        let s = Signer::new("TESTACCESSKEY000000000000", "TESTSECRETKEY000000000000");
        let got = s.token_at("pacz-cbps-dmz-stg", "f14d1f59-767b-404e-93b0-82454786d981", 1732761165);
        let want = "TESTACCESSKEY000000000000:XVLxWj6SoN9LF9ufU-6dP1tHbGE=:eyJzY29wZSI6InBhY3otY2Jwcy1kbXotc3RnOmYxNGQxZjU5LTc2N2ItNDA0ZS05M2IwLTgyNDU0Nzg2ZDk4MSIsImRlYWRsaW5lIjoxNzMyNzYxMTY1fQ==";
        assert_eq!(got, want);
    }

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{:02x}", x)).collect()
    }
}
