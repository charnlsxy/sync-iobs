# sync-iobs

iobs 文件双向同步命令行工具：上传 / 下载、实时进度、自定义 key 与文件名后缀、内外网地址可配。

- 语言：Rust（edition 2021）
- 依赖：`ureq` + `native-tls`（SHA1 / HMAC / base64 / JSON / INI / CLI / 进度条全部自实现）
- 目标：Windows 10+（`x86_64-pc-windows-gnu` 静态链 CRT，无需 VC 运行库）

## 启动

`dist\` 目录里已放好 `sync-iobs.exe` + `iobs.ini`，整目录复制到任意机器即可运行。

| 想做的事 | 操作 |
|---|---|
| 开图形界面 | 双击 `dist\run-gui.bat`（等价于不带参数运行 `sync-iobs.exe`）；**首次打开无需 ak/sk，打开后在「连接凭据」里填** |
| 用命令行 | 双击 `dist\run-cli.bat`，窗口里敲 `sync-iobs upload ...` 等命令（命令行模式仍要求 ak/sk，否则报错退出） |
| 只拷一个 exe | 可以，但 `iobs.ini` 要放 exe 同目录或 `%APPDATA%\sync-iobs\` |

## 图形界面

**双击 `sync-iobs.exe`**（不带任何参数）即启动界面，不用记命令：

- **上传**：浏览选择本地文件 → 填 key（留空用文件名）→ 填后缀（留空用配置里的 `file_ext`）→ 开始上传
- **下载**：填 key → 选保存目录（留空存当前目录）→ 开始下载
- **环境切换**：右上角下拉，直接切换外网/内网（读的是配置文件里的环境段）
- **配置来源**：标题下方会显示**当前实际加载的 `iobs.ini` 路径**以及**凭据是从哪来的**（配置文件 / 命令行 / 环境变量 / 都没配）。
  程序启动时会按 `--config` → 当前目录 → exe 同目录 → `%APPDATA%\sync-iobs\` 的顺序自动查找同名 `iobs.ini`，
  所以把 exe 放进一个已有凭据的目录时会直接沿用那份配置——这一行就是让你看清这件事的地方。
- **连接凭据**：界面顶部可填 access_key / secret_key（掩码显示），点「保存」写回 `iobs.ini`；**没有填也能直接打开界面**，之后在界面里补上即可
- **任务列表**：实时进度条、已传/总量、速度、状态；完成后可一键复制下载地址、打开所在目录

界面用 egui 绘制（纯 Rust，系统原生窗口），会自动加载系统中文字体，GUI 模式下自动隐藏控制台黑框。

```bash
# 默认编译（带 GUI）
cargo build --release

# 只要 CLI，零 GUI 依赖（产物约 0.7MB）
cargo build --release --no-default-features

# 静态链接 CRT（产物可直接拷贝到目标机器）
RUSTFLAGS="-C target-feature=+crt-static" cargo build --release --target x86_64-pc-windows-gnu
```

要求：Windows 10+；GUI 依赖 OpenGL（一般机器都有）。

## 配置

配置文件 `iobs.ini`，查找顺序：`--config` 指定 > 当前目录 > exe 同目录 > `%APPDATA%\sync-iobs\`。
也可用 `sync-iobs init-config` 生成一份模板。

```ini
[common]
bucket           = your-bucket   ; 必填
access_key       = ...           ; 必填
secret_key       = ...           ; 必填
small_file_limit = 10MB    ; 小于该值走小文件直传
chunk_size       = 5MB     ; 分片大小
timeout          = 60
retry            = 3
token_ttl        = 600     ; token 有效期（秒）

# 环境段名字可自由增删，运行时用 --env <名称> 选择；地址按你的实际环境填
[outer]
base_url = https://iobs.example.com

[inner]
base_url = https://iobs-inner.example.com
```

> `base_url`、`bucket`、`access_key`、`secret_key` **都没有内置默认值**，必须配置后程序才能工作；
> 缺哪一项都会在启动时明确报错并提示补齐方式。

`--env` 后面跟的是 section 名，可任意增删环境段；环境段内的配置优先于 `[common]`。

优先级：**命令行 > 环境变量 > 环境段 > common 段 > 内置默认**

环境变量：`IOBS_ENV` `IOBS_BASE_URL` `IOBS_BUCKET` `IOBS_AK` `IOBS_SK`

## 用法

```bash
# 上传（key 默认取文件名）
sync-iobs upload ./a.zip

# 自定义 key 与服务端文件名后缀（--ext 只改服务端保存名，不改 key）
sync-iobs upload ./a.zip --key cz20251130_test --ext png

# 强制分片 / 指定分片大小
sync-iobs upload ./big.iso --force-multipart --chunk 10MB

# 下载（--resume 断点续传）
sync-iobs download cz20251130_test --out ./recv/a.zip --resume

# 只打印带 token 的下载地址
sync-iobs url cz20251130_test

# 切换环境
sync-iobs upload ./a.zip --env inner
```

key 支持占位符：`{file}` 完整文件名、`{name}` 主文件名、`{ext}` 扩展名、`{ts}` 时间戳、`{date}` 日期。

## 服务端的两个硬约束（实测）

1. **分片大小下限 5MB**：小于 5MB 会返回 `400 EntityTooSmall`；程序检测到会自动纠正为 5MB 并警告。
2. **bucket 会拒绝部分文件后缀**：例如 `.bin` 会返回
   `402 refused bin file type`。用 `--ext` 或配置里的 `file_ext` 指定服务端保存的后缀。

错误响应体会直接打印出来（例如 `HTTP 402 {"code":402,...}`），4xx 不做无谓重试，只有 5xx/429 才重试。

## iobs 协议（逆向自 Java 实现，已实测）

| 用途 | 方法 | 路径 |
|---|---|---|
| 小文件直传 | POST | `/upload/{bucket}/{key}`（multipart：`token` + `file`） |
| 分片初始化 | POST | `/initUploadPart/{bucket}/{key}?token=&fileSize=&fileName=` → 纯文本 uploadId |
| 上传分片 | POST | `/uploadPart/{bucket}/{key}?token=&uploadId=&partNumber=`（1 起）→ JSON `hash` |
| 合并分片 | POST | `/completeUpload/{bucket}/{key}?token=&uploadId=`（form `partMap={"1":"hash",...}`） |
| 下载 | GET | `/download/{bucket}/{key}?token=` |

token = `ak : base64url(HMAC-SHA1(SHA1(sk), base64url({"scope":"bucket:key","deadline":ts}))) : message`，默认 600 秒有效；
长传时程序会在剩余不足 60 秒时自动续签。

## 测试

```bash
cargo test    # 含与 Java 参考 token 的逐字节比对
```
