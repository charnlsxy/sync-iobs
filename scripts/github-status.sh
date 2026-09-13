#!/usr/bin/env bash
# ============================================================
# 检查 GitHub Actions 构建结果
#
# 用法：
#   export GH_TOKEN=ghp_xxx
#   ./scripts/github-status.sh owner/repo                 # 看最近一次运行
#   ./scripts/github-status.sh owner/repo <run_id>        # 看指定运行
#   ./scripts/github-status.sh owner/repo <run_id> --tag v0.1.0   # 成功后打 tag 出 Release
#   WATCH=1 ./scripts/github-status.sh owner/repo         # 持续轮询直到结束
# ============================================================
set -euo pipefail

API="https://api.github.com"
FULL="${1:?用法: github-status.sh owner/repo [run_id] [--tag vX.Y.Z]}"
RUN_ID="${2:-}"
TAG=""
[ "${3:-}" = "--tag" ] && TAG="${4:-}"
WATCH="${WATCH:-0}"
TIMEOUT_MIN="${TIMEOUT_MIN:-40}"

: "${GH_TOKEN:?未设置 GH_TOKEN}"

api() {
  curl -sS -X "${1}" "$API${2}" \
    -H "Authorization: Bearer $GH_TOKEN" \
    -H "Accept: application/vnd.github+json" \
    -H "X-GitHub-Api-Version: 2022-11-28" \
    ${3:+-d "$3"}
}

PY=$(command -v python3 || command -v python)

# 取最近一次运行
if [ -z "$RUN_ID" ]; then
  RUN_ID=$(api GET "/repos/$FULL/actions/runs?per_page=1" \
    | "$PY" -c "import sys,json;r=json.load(sys.stdin).get('workflow_runs',[]);print(r[0]['id'] if r else '')")
  [ -n "$RUN_ID" ] || { echo "没有任何 Actions 运行记录"; exit 1; }
fi

echo "==> 运行详情：https://github.com/$FULL/actions/runs/$RUN_ID"

deadline=$(( $(date +%s) + TIMEOUT_MIN * 60 ))
while :; do
  RUN=$(api GET "/repos/$FULL/actions/runs/$RUN_ID")
  STATUS=$(printf '%s' "$RUN" | "$PY" -c "import sys,json;print(json.load(sys.stdin).get('status',''))")
  CONCL=$(printf '%s' "$RUN" | "$PY" -c "import sys,json;print(json.load(sys.stdin).get('conclusion') or '')")

  JOBS_JSON=$(api GET "/repos/$FULL/actions/runs/$RUN_ID/jobs?per_page=50")
  printf '%s' "$JOBS_JSON" | "$PY" -c "
import sys,json
jobs=json.load(sys.stdin).get('jobs',[])
print('  当前各任务状态：')
for j in jobs:
    icon={'success':'OK ','failure':'FAIL','cancelled':'CANCEL','skipped':'-','in_progress':'..','queued':'..'}.get(j['status']=='completed' and j['conclusion'] or j['status'],'..')
    print(f\"    [{icon:<6}] {j['name']:<38} {j['status']}/{j.get('conclusion') or '-'}\")
"

  if [ "$STATUS" = "completed" ]; then break; fi
  if [ "$WATCH" != "1" ]; then
    echo "  （未加 WATCH=1，仅报告当前状态）"
    exit 0
  fi
  if [ "$(date +%s)" -gt "$deadline" ]; then echo "  超时，仍在运行中"; exit 1; fi
  sleep 15
done

echo
if [ "$CONCL" = "success" ]; then
  echo "==> 构建成功"
else
  echo "==> 构建失败（$CONCL），下面是失败步骤的日志尾部："
  printf '%s' "$JOBS_JSON" | "$PY" -c "
import sys,json
for j in json.load(sys.stdin).get('jobs',[]):
    if j.get('conclusion')=='failure':
        for s in j.get('steps',[]):
            if s.get('conclusion')=='failure':
                print(f\"  job={j['name']}  step={s['name']}\")
"
  echo "  完整日志：https://github.com/$FULL/actions/runs/$RUN_ID"
  exit 1
fi

# 列出产物
echo
echo "==> 本次产物（Artifacts）："
api GET "/repos/$FULL/actions/runs/$RUN_ID/artifacts" | "$PY" -c "
import sys,json
for a in json.load(sys.stdin).get('artifacts',[]):
    print(f\"    {a['name']:<24} {a['size_in_bytes']/1048576:.2f} MB\")
"

# 打 tag 出 Release
if [ -n "$TAG" ]; then
  echo
  echo "==> 打 tag $TAG 触发 Release"
  api POST "/repos/$FULL/git/refs" "{\"ref\":\"refs/tags/$TAG\",\"sha\":\"$(api GET /repos/$FULL/git/ref/heads/main | "$PY" -c "import sys,json;print(json.load(sys.stdin)['object']['sha'])")\"}" >/dev/null 2>&1 || echo "    tag 可能已存在"
  echo "    已触发，稍后查看：https://github.com/$FULL/releases"
fi
