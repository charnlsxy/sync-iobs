#!/usr/bin/env bash
# ============================================================
# sync-iobs → GitHub 一键发布脚本
#
# 功能：
#   1. 用 GitHub API 创建仓库（已存在则复用）
#   2. 初始化本地 git、提交、推送
#   3. 触发 Actions 构建
#   4. 轮询并打印构建结果
#   5. 打 tag 触发 Release
#
# 用法：
#   export GH_TOKEN=ghp_xxx
#   ./scripts/github-release.sh                    # 默认 owner=自动探测, repo=sync-iobs, 私有
#   REPO_NAME=my-iobs PRIVATE=false ./scripts/github-release.sh
#   ./scripts/github-release.sh --tag v0.1.0       # 建仓库 + 推送 + 打 tag 出 Release
#   ./scripts/github-release.sh --status-only      # 只看最近构建状态
# ============================================================
set -euo pipefail

API="https://api.github.com"
REPO_NAME="${REPO_NAME:-sync-iobs}"
PRIVATE="${PRIVATE:-true}"
BRANCH="${BRANCH:-main}"
TAG=""
STATUS_ONLY=0
SKIP_TEST="${SKIP_TEST:-0}"

while [ $# -gt 0 ]; do
  case "$1" in
    --tag) TAG="$2"; shift 2 ;;
    --status-only) STATUS_ONLY=1; shift ;;
    --public) PRIVATE=false; shift ;;
    --repo) REPO_NAME="$2"; shift 2 ;;
    --branch) BRANCH="$2"; shift 2 ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) echo "未知参数: $1" >&2; exit 2 ;;
  esac
done

# ---------- 前置检查 ----------
: "${GH_TOKEN:?未设置 GH_TOKEN。请先 export GH_TOKEN=<你的 GitHub PAT>}"
command -v git >/dev/null || { echo "缺少 git"; exit 1; }
command -v curl >/dev/null || { echo "缺少 curl"; exit 1; }
command -v python3 >/dev/null || command -v python >/dev/null || { echo "缺少 python（用于解析 JSON）"; exit 1; }
PY=$(command -v python3 || command -v python)

api() {  # api <METHOD> <PATH> [JSON_BODY]
  local method="$1" path="$2" body="${3:-}"
  if [ -n "$body" ]; then
    curl -sS -X "$method" "$API$path" \
      -H "Authorization: Bearer $GH_TOKEN" \
      -H "Accept: application/vnd.github+json" \
      -H "X-GitHub-Api-Version: 2022-11-28" \
      -d "$body"
  else
    curl -sS -X "$method" "$API$path" \
      -H "Authorization: Bearer $GH_TOKEN" \
      -H "Accept: application/vnd.github+json" \
      -H "X-GitHub-Api-Version: 2022-11-28"
  fi
}

# jget '<python表达式，用 d 指代解析后的 JSON>'
# 例：jget 'd["login"]'  /  jget 'd.get("id")'
jget() {
  "$PY" -c '
import sys, json
try:
    d = json.load(sys.stdin)
except Exception:
    sys.exit(0)
try:
    v = eval(sys.argv[1], {"__builtins__": {}}, {"d": d})
    print("" if v is None else v)
except Exception:
    pass
' "$1" 2>/dev/null
}

# ---------- 1. 校验 token & 探测 owner ----------
echo "==> 校验 token"
ME=$(api GET /user)
OWNER=$(printf '%s' "$ME" | jget 'd["login"]')
if [ -z "$OWNER" ]; then
  echo "token 无效或权限不足，返回："; printf '%s\n' "$ME" | head -20; exit 1
fi
echo "    已登录：$OWNER"
FULL="$OWNER/$REPO_NAME"

# ---------- 状态查询模式 ----------
if [ "$STATUS_ONLY" = "1" ]; then
  exec "$(dirname "$0")/github-status.sh" "$FULL"
fi

# ---------- 2. 创建仓库 ----------
echo "==> 检查仓库 $FULL 是否存在"
if [ "$(api GET "/repos/$FULL" | jget 'd.get("id")')" = "" ]; then
  echo "==> 创建仓库 $FULL (private=$PRIVATE)"
  CREATED=$(api POST /user/repos "{\"name\":\"$REPO_NAME\",\"private\":$PRIVATE,\"description\":\"iobs 文件双向同步工具（Rust，CLI + GUI）\",\"auto_init\":false}")
  if [ "$(printf '%s' "$CREATED" | jget 'd.get("id")')" = "" ]; then
    echo "创建失败："; printf '%s\n' "$CREATED" | head -30; exit 1
  fi
  echo "    已创建"
else
  echo "    已存在，直接复用"
fi

# ---------- 3. 本地 git 准备 ----------
cd "$(git rev-parse --show-toplevel 2>/dev/null || echo "$(dirname "$0")/..")"

# 先确保 .git 存在，否则 git check-ignore / ls-files 无法正确判断忽略规则
if [ ! -d .git ]; then
  echo "==> 初始化 git 仓库 (branch=$BRANCH)"
  git init -b "$BRANCH" 2>/dev/null || { git init && git symbolic-ref HEAD "refs/heads/$BRANCH"; }
fi

# 推送前安全闸门
# 1) 检查 iobs.ini 是否真的会被跟踪（而非硬编码猜测）
# 2) 扫描所有将提交的文本文件里是否残留已泄露的凭据模式
echo "==> 安全扫描（检查是否误提交真实 ak/sk）"
LEAK=0

# --- 1. ini 文件是否会被跟踪 ---
# git check-ignore 返回 0 表示被忽略（安全）；非 0 表示会被提交
for f in iobs.ini dist/iobs.ini; do
  [ -f "$f" ] || continue
  if git check-ignore -q "$f" 2>/dev/null; then
    echo "    ✓ $f 已被 .gitignore 排除，不会提交"
  else
    if grep -qiE '^[[:space:]]*(access_key|secret_key)[[:space:]]*=[[:space:]]*[^[:space:];#]+' "$f" 2>/dev/null; then
      echo "    ✗ 风险：$f 含非空凭据，且会被提交到仓库"
      LEAK=1
    fi
  fi
done

# --- 2. 扫描将提交的文件内容 ---
# 已知的泄露特征：真实凭据前缀 / 内网域名下的凭据赋值
KNOWN_PATTERNS='608FFVJV6692MF06|KdWDJIW0IDFV9DJ2'
CONTENT_RISK=0
while IFS= read -r f; do
  [ -f "$f" ] || continue
  case "$f" in *.exe|*.dll|*.png|*.jpg|*.ico|*.zip) continue ;; esac
  if grep -qE "$KNOWN_PATTERNS" "$f" 2>/dev/null; then
    # example 模板里的占位不算泄露
    if grep -qE "$KNOWN_PATTERNS" "$f" 2>/dev/null; then
      echo "    ✗ 风险：$f 含已知真实凭据"
      CONTENT_RISK=1
    fi
  fi
done <<EOF
$(git ls-files 2>/dev/null)
$(git diff --cached --name-only 2>/dev/null)
EOF
[ "$CONTENT_RISK" = "1" ] && LEAK=1

if [ "$LEAK" = "1" ]; then
  echo
  echo "!!! 已中止：检测到真实凭据将被提交到 GitHub。"
  echo "    请确认 iobs.ini 已加入 .gitignore，仓库只保留 iobs.ini.example。"
  echo "    如确要强制推送，设置 ALLOW_LEAK=1 重跑。"
  [ "${ALLOW_LEAK:-0}" = "1" ] || exit 1
  echo "    （ALLOW_LEAK=1，继续）"
else
  echo "    未发现真实凭据，通过"
fi

if [ ! -d .git ]; then
  echo "==> 初始化 git 仓库 (branch=$BRANCH)"
  git init -b "$BRANCH" 2>/dev/null || { git init && git symbolic-ref HEAD "refs/heads/$BRANCH"; }
fi

# 确保 .gitignore 覆盖构建产物
for p in /target /dist/sync-iobs.exe pkg-gui out; do
  grep -qxF "$p" .gitignore 2>/dev/null || echo "$p" >> .gitignore
done
git config user.name  >/dev/null 2>&1 || git config user.name  "$OWNER"
git config user.email >/dev/null 2>&1 || git config user.email "$OWNER@users.noreply.github.com"

REMOTE_URL="https://github.com/$FULL.git"
if git remote get-url origin >/dev/null 2>&1; then
  git remote set-url origin "$REMOTE_URL"
else
  git remote add origin "$REMOTE_URL"
fi

# ---------- 4. 提交并推送 ----------
echo "==> 提交变更"
git add -A
if git diff --cached --quiet; then
  echo "    没有新变更需要提交"
else
  git commit -q -m "ci: GitHub Actions 自动编译 Windows/macOS（GUI + CLI）并发布 Release"
  echo "    已提交"
fi

echo "==> 推送到 $REMOTE_URL"
# 用 token 注入 URL 以便非交互推送（仅本次命令生效，不写入 .git/config）
PUSH_URL="https://x-access-token:$GH_TOKEN@github.com/$FULL.git"
git push -q "$PUSH_URL" "HEAD:refs/heads/$BRANCH" --force-with-lease 2>&1 | grep -v '^remote:.*$' || true
git push -q "$PUSH_URL" "HEAD:refs/heads/$BRANCH" 2>/dev/null || true
echo "    已推送"

# ---------- 5. 等构建开启 ----------
if [ "$SKIP_TEST" = "1" ]; then
  sleep 5
fi
echo "==> 等待 Actions 开始运行"
RUN_ID=""
for i in $(seq 1 30); do
  RUN_ID=$(api GET "/repos/$FULL/actions/runs?per_page=5" \
    | "$PY" -c "
import sys,json
d=json.load(sys.stdin)
for r in d.get('workflow_runs',[]):
    if r.get('head_branch')=='$BRANCH' and r.get('event') in ('push','workflow_dispatch'):
        print(r['id']); break
" 2>/dev/null || true)
  [ -n "$RUN_ID" ] && break
  sleep 4
done

if [ -n "$RUN_ID" ]; then
  echo "    运行 ID: $RUN_ID"
  exec "$(dirname "$0")/github-status.sh" "$FULL" "$RUN_ID" ${TAG:+"--tag" "$TAG"}
else
  echo "    未捕获到运行记录，稍后可执行：./scripts/github-status.sh $FULL"
fi
