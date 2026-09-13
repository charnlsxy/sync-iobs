# sync-iobs 项目长期记忆

## 项目目标
Rust 写的 iobs 文件双向同步命令行工具（上传/下载 + 实时进度 + 自定义 key 与后缀 + 内外网 URL 可配）。

## iobs 协议要点（逆向自 F:\project\loadfile 的 Java 实现）

- token = `accessKey:signStr:messageStr`
  - `messageStr` = base64url(`{"scope":"<bucket>:<key>","deadline":<秒级时间戳>}`)，带 padding
  - `signStr` = base64url(HMAC-SHA1(key = SHA1(secretKey) 的 20 字节, msg = messageStr))
  - 默认有效期 600 秒；token 原样拼进 query（冒号不转义）
- 端点（HOST 可换，二选一：外网 `https://stg-iobs.pingan.com.cn` / 内网 `https://stg-iobs-sf.paic.com.cn`；bucket 默认 `pacz-cbps-dmz-stg`；ak/sk 见 Java 源码 Uploader.java，不入库）
  - 小文件：`POST /upload/{b}/{k}`，multipart/form-data，字段 token + file（filename 可自定义后缀）
  - 分片：`POST /initUploadPart/{b}/{k}?token&fileSize&fileName` → uploadId（纯文本）
  - `POST /uploadPart/{b}/{k}?token&uploadId&partNumber`（1 起，body 原始字节，响应取 `hash`）
  - `POST /completeUpload/{b}/{k}?token&uploadId`，form `partMap=urlencode({"1":"hash",...})`
  - 下载：`GET /download/{b}/{k}?token`
- 大小阈值 10MB；分片默认 5MB。

## 服务端硬约束（实测）
- 分片大小下限 **5MB**（小于则 `400 EntityTooSmall`）；程序自动纠正并警告。
- bucket 拒绝部分后缀，如 `.bin` → `402 refused bin file type`；必须用 `--ext` 或配置 `file_ext`。
- 错误响应体是 JSON（如 `{"code":402,"info":"..."}`），4xx 不重试，仅 5xx/429 重试。

## 约定
- 依赖最小化：仅 ureq + native-tls，SHA1/HMAC/base64/JSON/CLI/进度条全部自实现。
- ureq 2.x 的 native-tls **只是适配器**，必须显式 `AgentBuilder::tls_connector(Arc::new(native_tls::TlsConnector::new()?))`，否则 https 报 "no TLS backend is configured"。
- 目标平台 Windows 10+，编译加 `-C target-feature=+crt-static`。
- 不用 async 运行时。

## 本机构建环境（无 MSVC、GitHub 不可达）
- ~~无 MSVC~~ **已装好 VS Build Tools 2022**，在 `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`（MSVC 14.44）。MSVC 宿主 Rust 1.98.1 在 `F:/tools/rust`；编译：`PATH=/f/tools/rust/bin; CARGO_HOME=F:\tools\cargo-home; cargo build --release`
- GNU 备用链：Rust 1.98.1-gnu 在 `F:/tools/rust-gnu`；MinGW-w64 在 `F:/tools/mingw64`
- 静态产物（GNU）：`RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target x86_64-pc-windows-gnu`

## CI / 发布（GitHub Actions）
- workflow `.github/workflows/release.yml`：test(win/mac/linux) → build-windows(MSVC+静态CRT) → build-macos(arm64+x86_64 矩阵) → release(tag `v*` 触发)
- 一键脚本 `scripts/github-release.sh`（API 建私有仓库+推送+轮询构建）、`scripts/github-status.sh`（查状态/日志/产物）
- **安全红线**：`iobs.ini`（含真实 ak/sk）必须在 `.gitignore` 里，仓库与 Release 只放 `iobs.ini.example` 脱敏模板；推送脚本有凭据泄露闸门
- **runner 坑**：`macos-13` 已退役（2025-12-08），Intel 用 `macos-15-intel`，Apple Silicon 用 `macos-14`
- CI 用 `x86_64-pc-windows-msvc`；本机 `dist/` 产物不入库
