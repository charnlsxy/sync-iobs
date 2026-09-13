# GitHub Actions 自动编译说明

> 已配置完成，**只差一个 GitHub token** 就能跑通全流程。

## 一、你会得到什么

推送代码 → GitHub 自动编译 → 产出四个平台的二进制：

| 产物 | 平台 | 说明 |
|---|---|---|
| `sync-iobs-windows-x86_64-gui.zip` | Windows 64 位 | exe + 配置模板 + 双击启动 bat，**静态链接 CRT，拷走即用** |
| `sync-iobs-windows-x86_64-cli.exe` | Windows 64 位 | 纯命令行版，零 GUI 依赖，体积小 |
| `sync-iobs-macos-arm64-gui.zip` | Apple Silicon（M 系列） | 标准 `.app` 包，双击运行 |
| `sync-iobs-macos-arm64-cli` | Apple Silicon | 命令行单文件 |
| `sync-iobs-macos-x86_64-gui.zip` | Intel Mac | 同上 |
| `sync-iobs-macos-x86_64-cli` | Intel Mac | 同上 |

打 tag（如 `v0.1.0`）时，自动创建 GitHub Release，附上全部产物 + `SHA256SUMS.txt` 校验和。

## 二、触发方式

**只在打 tag 时跑完整流水线**，避免同一 commit 被跑两遍（main 推送 + tag 各一次）。

| 动作 | 结果 |
|---|---|
| 推 `v*` tag | 跑完整流程：测试 → 编译三平台 → **发布 Release** |
| 提 Pull Request | 跑测试 + 编译校验，产物存为 Actions Artifacts（**不发布**） |
| 手动 | Actions 页面点 `Run workflow`，可用于不发版时验证能否编译 |

**注意**：推送代码到 `main` **不会**触发构建。要出产物就打个 tag：

```bash
git tag v0.1.1
git push origin v0.1.1
```

> tag 名必须与 `Cargo.toml` 里的 `version` 一致（如 tag `v0.1.1` ↔ `version = "0.1.1"`），
> 不一致时 Release 会直接失败并提示，防止发错版本。

## 三、三平台测试

`test` job 会在 Ubuntu / Windows / macOS 上跑 `cargo test` 和 CLI 编译，验证跨平台兼容性 —— 通过后才会进入打包阶段。

## 四、安全设计

⚠️ **你的 `iobs.ini` 含真实平安内部 ak/sk，已做隔离处理：**

- `iobs.ini` 和 `dist/iobs.ini` 已加入 `.gitignore`，**不会进入 git 历史**
- 仓库和 Release 包里只放脱敏模板 `iobs.ini.example`（ak/sk 留空）
- 发布脚本内置**推送前安全闸门**，检测到真实凭据会直接中止
- 用户拿到包后，在 GUI「连接凭据」里填自己的 ak/sk 即可

## 五、跑起来（需要你提供 token）

### 1. 生成 token

GitHub → Settings → Developer settings → Personal access tokens → **Fine-grained tokens**

需要的权限：
- **Repository access**：可以设为 `All repositories`，或建好仓库后指定
- **Permissions**：
  - `Administration: Read and write` — 建仓库
  - `Contents: Read and write` — 推代码、打 tag
  - `Workflows: Read and write` — 提交 workflow 文件
  - `Metadata: Read` — 自动附带

> 生成后请**只在本机使用**，用完整流程跑通后建议撤销重建。

### 2. 执行一键发布

```bash
cd /f/game/hy-test/sync-iobs

export GH_TOKEN=<你的 token>

# 建私有仓库 + 推代码 + 触发构建 + 检查结果
./scripts/github-release.sh

# 想同时出 Release：
./scripts/github-release.sh --tag v0.1.0
```

### 3. 只查构建状态

```bash
export GH_TOKEN=<你的 token>

# 看最近一次运行
./scripts/github-status.sh <owner>/sync-iobs

# 持续轮询到结束
WATCH=1 ./scripts/github-status.sh <owner>/sync-iobs

# 查看指定运行
./scripts/github-status.sh <owner>/sync-iobs <run_id>
```

构建失败时脚本会自动指出**哪个 job 的哪一步**失败了，并给出日志链接。

## 六、本机编译 vs CI 编译

你本机是 `x86_64-pc-windows-gnu` + MinGW（因为没装 MSVC）。CI 上环境齐全，改用 **`x86_64-pc-windows-msvc`** —— 官方工具链，产物兼容性和性能都更好，因此**本机的 `dist/sync-iobs.exe` 已加入 gitignore**，不再入库，统一由 CI 产出。

## 七、可调项（编辑 `.github/workflows/release.yml`）

| 想改什么 | 改哪里 |
|---|---|
| 加 Linux 构建 | `build-windows` / `build-macos` 旁再复制一个 job，`runs-on: ubuntu-latest` |
| 只编 CLI、不要 GUI | 删掉 workflow 里 `Build GUI` 那一步 |
| 改 Rust 版本 | `dtolnay/rust-toolchain@stable` → `@1.80.0` 之类 |
| Release 设为草稿 | `release` job 里 `draft: false` → `true` |

## 八、runner 版本说明（容易踩的坑）

- **`macos-13` 已于 2025-12-08 退役**，用它跑构建会直接失败。Intel Mac 目标已改用 **`macos-15-intel`**（官方最后一个 x86_64 镜像，预计支持到 2027-08）。
- Apple Silicon 用 `macos-14`（arm64）。
- 2027-08 之后 GitHub Actions 将不再提供 Intel macOS runner，届时如需继续支持 Intel Mac，得改用自托管 runner 或做交叉编译。
- macOS 产物**未做代码签名**，用户首次打开会被 Gatekeeper 拦截。解决方法：右键 → 打开，或执行
  `xattr -cr SyncIOBS.app`。如需正式签名，要配置 Apple Developer 证书（付费账号）。

