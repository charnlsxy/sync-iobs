#!/usr/bin/env python3
"""
用 GitHub Git Data API 推送本地文件到远程分支（绕过 git push）。

适用场景：本机 `git push` 因网络/代理问题反复失败（SSL unexpected eof / CONNECT tunnel 502）。

用法：
    export GH_TOKEN=ghp_xxx
    python scripts/gh-api-push.py --repo owner/name --branch main \
        --message "fix: xxx" --files Cargo.toml Cargo.lock .github/workflows/release.yml

    # 用当前 HEAD 的提交信息，并自动带上所有已跟踪且与远程有差异的文件
    python scripts/gh-api-push.py --repo owner/name --branch main --all-tracked

原理：
    1. POST /git/blobs                     为每个文件建 blob
    2. POST /git/trees  (带 base_tree)      基于远程当前 tree 建新 tree
    3. POST /git/commits                    建 commit（parent = 远程当前 HEAD）
    4. PATCH /git/refs/heads/{branch}       快进分支引用（force=False，避免覆盖他人提交）

注意：这是「整文件覆盖」式推送，不适合需要保留 merge 历史的场景；
      若远程 HEAD 已不是本地基线，脚本会拒绝执行（防止覆盖别人提交）。
"""
import argparse
import base64
import json
import os
import sys
import subprocess
import time
import urllib.error
import urllib.request

API = "https://api.github.com"


def api(token, method, path, body=None, retries=5):
    last = None
    for i in range(retries):
        try:
            data = json.dumps(body).encode() if body is not None else None
            req = urllib.request.Request(
                API + path,
                method=method,
                data=data,
                headers={
                    "Authorization": "token " + token,
                    "Accept": "application/vnd.github+json",
                    "User-Agent": "gh-api-push",
                },
            )
            if data:
                req.add_header("Content-Type", "application/json")
            with urllib.request.urlopen(req, timeout=120) as r:
                return json.loads(r.read().decode() or "{}")
        except urllib.error.HTTPError as e:
            detail = e.read().decode()[:300]
            # 404/422 是确定性错误，不重试
            if e.code in (401, 403, 404, 422):
                raise SystemExit(f"HTTP {e.code} on {method} {path}: {detail}")
            last = f"HTTP {e.code}: {detail}"
        except Exception as e:  # 网络抖动，重试
            last = f"{type(e).__name__}: {e}"
        if i < retries - 1:
            time.sleep(4)
    raise SystemExit(f"请求失败（重试 {retries} 次）：{method} {path} -> {last}")


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True, check=False).stdout


def git_paths(*args):
    """取 git 输出的路径列表。

    必须用 -z 且关闭 core.quotepath：否则非 ASCII 文件名（如「技术方案.md」）
    会被输出成 "\\346\\212\\200..." 这样的八进制转义串，导致后续读文件失败。
    """
    out = subprocess.run(["git", *args], capture_output=True, check=False).stdout
    return [p.decode("utf-8", "surrogateescape") for p in out.split(b"\0") if p]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--repo", required=True, help="owner/name")
    ap.add_argument("--branch", default="main")
    ap.add_argument("--message", help="提交信息；省略则用 git 最近一次提交的信息")
    ap.add_argument("--files", nargs="*", default=[], help="要推送的文件路径")
    ap.add_argument("--all-tracked", action="store_true", help="推送所有已跟踪文件（整树同步）")
    ap.add_argument("--expect-base", help="期望的远程基线 sha（默认为远程当前 HEAD）")
    args = ap.parse_args()

    token = os.environ.get("GH_TOKEN")
    if not token:
        raise SystemExit("未设置 GH_TOKEN")

    # 1. 远程当前 HEAD
    remote = api(token, "GET", f"/repos/{args.repo}/git/ref/heads/{args.branch}")
    base = remote["object"]["sha"]
    print(f"远程 {args.branch} HEAD: {base[:10]}")

    if args.expect_base and not base.startswith(args.expect_base):
        raise SystemExit(f"远程基线已变化（期望 {args.expect_base}，实际 {base[:10]}），请先拉取")

    # 2. 收集文件
    if args.all_tracked:
        files = git_paths("-c", "core.quotepath=false", "ls-files", "-z")
    else:
        files = args.files
    if not files:
        raise SystemExit("没有要推送的文件")

    items = []
    for f in files:
        p = f.replace("\\", "/")
        if not os.path.isfile(p):
            raise SystemExit(f"文件不存在：{p}")
        content = open(p, "rb").read()
        blob = api(token, "POST", f"/repos/{args.repo}/git/blobs",
                   {"content": base64.b64encode(content).decode(), "encoding": "base64"})
        print(f"  blob {p:<46} {len(content):>8} bytes -> {blob['sha'][:10]}")
        items.append({"path": p, "mode": "100644", "type": "blob", "sha": blob["sha"]})

    # 3. 建 tree
    base_tree = api(token, "GET", f"/repos/{args.repo}/git/commits/{base}")["tree"]["sha"]
    tree = api(token, "POST", f"/repos/{args.repo}/git/trees",
               {"base_tree": base_tree, "tree": items})

    # 4. 建 commit
    msg = args.message or git("log", "-1", "--pretty=%B") or "chore: update files"
    commit = api(token, "POST", f"/repos/{args.repo}/git/commits",
                 {"message": msg.strip(), "tree": tree["sha"], "parents": [base]})
    print(f"新 commit: {commit['sha'][:10]}")

    # 5. 快进分支（force=False：非快进会失败，避免覆盖）
    ref = api(token, "PATCH", f"/repos/{args.repo}/git/refs/heads/{args.branch}",
              {"sha": commit["sha"], "force": False})
    print(f"{args.branch} -> {ref['object']['sha'][:10]}")
    print("推送完成")


if __name__ == "__main__":
    main()
