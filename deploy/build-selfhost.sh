#!/usr/bin/env bash
#
# 打一个自建部署包：可执行文件 + deploy/ 模板 + README。
#
# 入口是 `pnpm run build:selfhost`。产物默认落在 <cargo target>/selfhost/
# （target/ 已经在 .gitignore 里，不需要额外加忽略规则）。
#
# 与官方 npx 包的区别：VK_SHARED_API_BASE 为空，不连任何云端服务。
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
# shellcheck source=deploy/common.sh
. "$SCRIPT_DIR/common.sh"

OUT_DIR_ARG=""
SKIP_BUILD=0

usage() {
  cat <<EOF
用法：$(basename "$0") [选项]

选项：
  --out PATH     产物目录（默认 \${CARGO_TARGET_DIR:-target}/selfhost）
  --skip-build   不重新编译，直接用已有的 release 产物打包
  -h, --help     显示本帮助

说明：
  VK_SHARED_API_BASE 默认为空（纯本地，不连云端）。要打一个连官方云端的包：
    VK_SHARED_API_BASE=https://api.vibekanban.com $(basename "$0")
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --out)        OUT_DIR_ARG="${2:?--out 需要一个路径}"; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    -h|--help)    usage; exit 0 ;;
    *)            usage >&2; vk_die "未知参数：$1" ;;
  esac
done

TARGET_DIR="${CARGO_TARGET_DIR:-target}"
case "$TARGET_DIR" in
  /*) ;;
  *) TARGET_DIR="$REPO_ROOT/$TARGET_DIR" ;;
esac
OUT_DIR="${OUT_DIR_ARG:-$TARGET_DIR/selfhost}"

OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in x86_64) ARCH="x64" ;; arm64|aarch64) ARCH="arm64" ;; esac
case "$OS" in darwin) OS="macos" ;; esac
PLATFORM="${OS}-${ARCH}"

vk_info "自建部署包：平台 ${PLATFORM}，产物目录 $OUT_DIR"

if [ "$SKIP_BUILD" -eq 0 ]; then
  vk_need_cmd cargo "先装 Rust 工具链：https://rustup.rs"
  vk_need_cmd pnpm "先装 pnpm：npm i -g pnpm"
  # VK_SHARED_API_BASE 不在这里写死：local-build.sh 已经是
  # "${VK_SHARED_API_BASE:-}"，默认空。调用方想连云端就自己传进来。
  vk_info "跑 local-build.sh（release 编译，会比较久）…"
  ( cd "$REPO_ROOT" && bash ./local-build.sh )
fi

BIN_SRC="$TARGET_DIR/release/server"
[ -f "$BIN_SRC" ] || vk_die "找不到可执行文件：$BIN_SRC
  先跑一次不带 --skip-build 的构建，或检查 CARGO_TARGET_DIR。"

STAGE="$OUT_DIR/vibe-kanban-selfhost-$PLATFORM"
rm -rf "$STAGE"
mkdir -p "$STAGE/deploy"

cp "$BIN_SRC" "$STAGE/vibe-kanban"
chmod +x "$STAGE/vibe-kanban"

# MCP 二进制是可选的：没有它主服务照常跑，只是本机 MCP 客户端接不上。
if [ -f "$TARGET_DIR/release/vibe-kanban-mcp" ]; then
  cp "$TARGET_DIR/release/vibe-kanban-mcp" "$STAGE/vibe-kanban-mcp"
  chmod +x "$STAGE/vibe-kanban-mcp"
fi

# deploy/ 整个带上，但不带打包脚本自己（部署机上用不到，还要求有 cargo）。
for item in launchd systemd common.sh install-macos.sh install-linux.sh \
            backup.sh restore.sh backup_test.sh server.example.json \
            Caddyfile.selfhost.example README.md; do
  if [ -e "$SCRIPT_DIR/$item" ]; then
    cp -R "$SCRIPT_DIR/$item" "$STAGE/deploy/"
  fi
done
# common.sh 是被 source 的库，不给执行位，免得有人直接跑它。
for s in install-macos.sh install-linux.sh backup.sh restore.sh backup_test.sh; do
  if [ -f "$STAGE/deploy/$s" ]; then chmod +x "$STAGE/deploy/$s"; fi
done
chmod 644 "$STAGE/deploy/common.sh" 2>/dev/null || true

# 顶层 README：三步走。详细说明在 deploy/README.md。
VERSION="$(cd "$REPO_ROOT" && git describe --tags --always 2>/dev/null || echo unknown)"
cat >"$STAGE/README.md" <<EOF
# vibe-kanban 自建部署包（${PLATFORM}）

构建于 $(date -u +%Y-%m-%dT%H:%M:%SZ)，源码版本 ${VERSION}。
**纯本地构建：VK_SHARED_API_BASE 为空，不连接任何云端服务。**

## 三步走

\`\`\`bash
# 1. 解压后进入本目录
# 2. 安装成开机自启的服务（团队模式，监听 8080）
./deploy/install-macos.sh --bin "\$PWD/vibe-kanban" --port 8080     # macOS
./deploy/install-linux.sh --bin "\$PWD/vibe-kanban" --port 8080     # Linux

# 3. 从日志里拿一次性初始化链接，浏览器打开创建管理员
grep -i setup <数据目录>/logs/stdout.log | tail -n 5
\`\`\`

只想本机自己用（免登录）：

\`\`\`bash
./vibe-kanban        # 个人模式，默认只监听 127.0.0.1
\`\`\`

## 接下来

- 完整的部署、局域网访问、HTTPS、备份恢复说明：\`deploy/README.md\`
- 配置样例：\`deploy/server.example.json\`（放到数据目录下改名为 \`server.json\`）
- 反向代理样例：\`deploy/Caddyfile.selfhost.example\`
- 备份：\`./deploy/backup.sh\`；恢复：\`./deploy/restore.sh <备份包>\`
EOF

TARBALL="$OUT_DIR/vibe-kanban-selfhost-$PLATFORM.tar.gz"
rm -f "$TARBALL"
tar -czf "$TARBALL" -C "$OUT_DIR" "$(basename "$STAGE")" \
  || vk_die "打包失败：$TARBALL"
tar -tzf "$TARBALL" >/dev/null 2>&1 || vk_die "生成的包读不回来：$TARBALL"

vk_ok "部署包：${TARBALL}（$(du -h "$TARBALL" 2>/dev/null | awk '{print $1}')）"
vk_ok "解包目录：$STAGE"
