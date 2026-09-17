#!/usr/bin/env bash
#
# 把 vibe-kanban 装成 macOS 的 launchd 用户代理（LaunchAgent）。
#
# 幂等：重复跑会先 bootout 旧的再 bootstrap 新的，不会留下两份。
# 可回滚：`./install-macos.sh --uninstall` 卸干净（**不删数据目录**）。
# 不用 sudo：装的是用户代理，全部落在 $HOME 下。用 sudo 跑会把 plist 和
# 数据文件的属主搞成 root，反而让服务起不来，所以脚本会直接拒绝。
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/common.sh
. "$SCRIPT_DIR/common.sh"

TEMPLATE="$SCRIPT_DIR/launchd/$VK_LABEL.plist"
PLIST="$HOME/Library/LaunchAgents/$VK_LABEL.plist"

BIN=""
PORT="8080"
HOST=""
MODE="team"
ORIGINS=""
DATA_DIR_ARG=""
DO_UNINSTALL=0
DRY_RUN=0

usage() {
  cat <<EOF
用法：
  $(basename "$0") --bin <可执行文件路径> [选项]
  $(basename "$0") --uninstall

选项：
  --bin PATH        vibe-kanban 可执行文件（必填，会校验存在且可执行）
  --mode MODE       personal | team           默认 team
  --host ADDR       监听地址                   默认 team=0.0.0.0，personal=127.0.0.1
  --port PORT       监听端口                   默认 $PORT
  --origins LIST    VK_ALLOWED_ORIGINS，逗号分隔。只在反代改写了 Host 头
                    或成员通过别名域名访问时才需要，详见 deploy/README.md
  --data-dir PATH   数据目录                   默认 ~/Library/Application Support/$VK_LABEL
  --dry-run         只打印将要写入的 plist，不动系统
  --uninstall       卸载服务（保留数据目录）
  -h, --help        显示本帮助
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --bin)       BIN="${2:?--bin 需要一个路径}"; shift 2 ;;
    --mode)      MODE="${2:?--mode 需要一个值}"; shift 2 ;;
    --host)      HOST="${2:?--host 需要一个值}"; shift 2 ;;
    --port)      PORT="${2:?--port 需要一个值}"; shift 2 ;;
    --origins)   ORIGINS="${2:?--origins 需要一个值}"; shift 2 ;;
    --data-dir)  DATA_DIR_ARG="${2:?--data-dir 需要一个路径}"; shift 2 ;;
    --dry-run)   DRY_RUN=1; shift ;;
    --uninstall) DO_UNINSTALL=1; shift ;;
    -h|--help)   usage; exit 0 ;;
    *)           usage >&2; vk_die "未知参数：$1" ;;
  esac
done

[ "$(uname -s)" = "Darwin" ] || vk_die "这个脚本只用于 macOS。Linux 请用 deploy/install-linux.sh。"
[ "$(id -u)" != "0" ] || vk_die "不要用 sudo / root 跑这个脚本。
  它装的是当前登录用户的 LaunchAgent，root 跑出来的服务读不到你的 ~/.ssh 与各家
  编码代理 CLI 的登录态，数据目录的属主也会变成 root。"

UID_NUM="$(id -u)"
DOMAIN="gui/$UID_NUM"

# ---- 卸载 ------------------------------------------------------------------

uninstall() {
  local found=0
  if launchctl print "$DOMAIN/$VK_LABEL" >/dev/null 2>&1; then
    found=1
    vk_info "停止并卸载服务…"
    launchctl bootout "$DOMAIN/$VK_LABEL" 2>/dev/null || true
  fi
  if [ -f "$PLIST" ]; then
    found=1
    rm -f "$PLIST"
    vk_ok "已删除 $PLIST"
  fi
  if [ "$found" -eq 0 ]; then
    vk_warn "服务未安装：既没有加载中的 ${VK_LABEL}，也没有 ${PLIST}。无需卸载。"
    return 0
  fi
  vk_ok "卸载完成。数据目录未删除。"
  return 0
}

if [ "$DO_UNINSTALL" -eq 1 ]; then
  uninstall
  exit 0
fi

# ---- 参数校验 --------------------------------------------------------------

[ -n "$BIN" ] || { usage >&2; vk_die "必须用 --bin 指定可执行文件"; }

case "$MODE" in
  personal|team) ;;
  *) vk_die "--mode 只能是 personal 或 team，收到：$MODE" ;;
esac

case "$PORT" in
  ''|*[!0-9]*) vk_die "--port 必须是数字，收到：$PORT" ;;
esac
[ "$PORT" -ge 1 ] && [ "$PORT" -le 65535 ] || vk_die "--port 超出范围：$PORT"
[ "$PORT" -ge 1024 ] || vk_die "--port $PORT 是特权端口，用户代理绑不上。请用 >=1024 的端口，
  真要用 80/443 就在前面放一层反向代理（见 deploy/Caddyfile.selfhost.example）。"

if [ -z "$HOST" ]; then
  if [ "$MODE" = team ]; then HOST="0.0.0.0"; else HOST="127.0.0.1"; fi
fi

# 个人模式免登录：谁能连上谁就有全部权限（能建工作区、跑命令、读仓库）。
# 这不是「配置得不好」，是「等于把 shell 开给整个网段」。
if [ "$MODE" = personal ] && [ "$HOST" != "127.0.0.1" ] && [ "$HOST" != "localhost" ] && [ "$HOST" != "::1" ]; then
  vk_die "拒绝把 personal 模式监听在 ${HOST}。
  个人模式没有登录，网络可达即全权——同网段任何人都能建工作区、在你的机器上跑命令。
  要多人用请改成 --mode team（强制登录），要本机用请去掉 --host。"
fi

# 二进制必须存在且可执行。少了这一条，launchd 会抱着一个不存在的路径
# 无限重启，而且 stdout.log 里什么都不会有——最难排查的一种「装好了但不工作」。
[ -e "$BIN" ] || vk_die "可执行文件不存在：$BIN"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
[ -f "$BIN" ] || vk_die "--bin 不是普通文件：$BIN"
[ -x "$BIN" ] || vk_die "可执行文件没有执行权限：$BIN
  修复：chmod +x '$BIN'"
if ! "$BIN" --version >/dev/null 2>&1 && ! "$BIN" --help >/dev/null 2>&1; then
  vk_warn "'$BIN --version' 与 '--help' 都跑不通。文件可能被 Gatekeeper 隔离，
  先手工跑一次 '$BIN' 看看，必要时 'xattr -d com.apple.quarantine $BIN'。"
fi

DATA_DIR="$(vk_resolve_data_dir "$DATA_DIR_ARG")"
vk_require_data_dir "$DATA_DIR" create
mkdir -p "$DATA_DIR/logs" || vk_die "创建日志目录失败：$DATA_DIR/logs"
vk_require_space "$DATA_DIR" 51200 "安装"

[ -f "$TEMPLATE" ] || vk_die "找不到 plist 模板：${TEMPLATE}（部署包不完整？）"

mkdir -p "$HOME/Library/LaunchAgents" || vk_die "创建 ~/Library/LaunchAgents 失败"

# ---- 渲染 plist ------------------------------------------------------------

# XML 转义：& < > 必须转，否则 plist 解析失败而 launchctl 只报一句 5:
# "Input/output error"。路径里带 & 的人不多但确实存在。
xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

# launchd 只给最小 PATH，而 vibe-kanban 要拉起 git 与编码代理 CLI。
RUNTIME_PATH="${PATH}"

RENDERED="$(mktemp -t vk-plist)" || vk_die "无法创建临时文件"
OLD_PLIST=""
trap 'rm -f "$RENDERED" ${OLD_PLIST:+"$OLD_PLIST"}' EXIT

# 先去掉模板顶部的说明性 XML 注释：它里面也有 __占位符__ 字样，留着会让
# 下面的「还有没替换的占位符」检查误报，而且注释里出现替换后的真实路径也没意义。
sed \
  -e "/^<!--$/,/^-->$/d" \
  -e "s|__BIN_PATH__|$(xml_escape "$BIN")|g" \
  -e "s|__DATA_DIR__|$(xml_escape "$DATA_DIR")|g" \
  -e "s|__MODE__|$(xml_escape "$MODE")|g" \
  -e "s|__HOST__|$(xml_escape "$HOST")|g" \
  -e "s|__PORT__|$(xml_escape "$PORT")|g" \
  -e "s|__ALLOWED_ORIGINS__|$(xml_escape "$ORIGINS")|g" \
  -e "s|__PATH__|$(xml_escape "$RUNTIME_PATH")|g" \
  "$TEMPLATE" >"$RENDERED"

if grep -q '__[A-Z_]*__' "$RENDERED"; then
  vk_die "plist 里还有没替换的占位符：$(grep -o '__[A-Z_]*__' "$RENDERED" | sort -u | tr '\n' ' ')"
fi

plutil -lint "$RENDERED" >/dev/null 2>&1 || vk_die "渲染出来的 plist 不合法（plutil -lint 失败）。
  多半是路径里有特殊字符。手工检查：plutil -lint $RENDERED"

if [ "$DRY_RUN" -eq 1 ]; then
  vk_info "--dry-run：以下内容将写入 $PLIST"
  cat "$RENDERED"
  exit 0
fi

# ---- 先停旧的，再查端口 ----------------------------------------------------

# 顺序很重要：先把我们自己的旧实例停掉，再查端口占用。反过来的话，
# 重装时永远会被「自己占着端口」挡下来，就不幂等了。
BOOTED_OUT=0
OLD_PLIST=""
if launchctl print "$DOMAIN/$VK_LABEL" >/dev/null 2>&1; then
  vk_info "检测到已安装的服务，先卸载旧实例（重装）…"
  # 先把旧 plist 留一份：后面任何一步失败都要能把它原样装回去，
  # 不能让一次失败的重装把好端端在跑的服务变成「没了」。
  if [ -f "$PLIST" ]; then
    OLD_PLIST="$(mktemp -t vk-plist-old)"
    cp "$PLIST" "$OLD_PLIST"
  fi
  launchctl bootout "$DOMAIN/$VK_LABEL" 2>/dev/null || true
  BOOTED_OUT=1
  # bootout 是异步的，给它一点时间释放端口。
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    vk_port_in_use "$PORT" || break
    sleep 0.3
  done
fi

# 把旧实例原样装回去。用在「已经 bootout 但新配置装不上」的路径上。
restore_old_service() {
  [ "$BOOTED_OUT" -eq 1 ] || return 0
  [ -n "$OLD_PLIST" ] || return 0
  [ -f "$OLD_PLIST" ] || return 0
  cp "$OLD_PLIST" "$PLIST" 2>/dev/null || return 0
  if launchctl bootstrap "$DOMAIN" "$PLIST" 2>/dev/null; then
    vk_warn "已把安装前正在运行的旧实例恢复回去。"
  else
    vk_warn "旧实例没能自动恢复。手工恢复：launchctl bootstrap $DOMAIN $PLIST"
  fi
}

if vk_port_in_use "$PORT"; then
  holder="$(vk_port_holder "$PORT")"
  restore_old_service
  vk_die "端口 $PORT 已被占用${holder:+（${holder}）}。
  换一个端口（--port）或先停掉占用方。新配置未安装。"
fi

# ---- 安装并启动 ------------------------------------------------------------

cp "$RENDERED" "$PLIST" || vk_die "写入 $PLIST 失败"
chmod 644 "$PLIST"
vk_ok "已写入 $PLIST"

if ! launchctl bootstrap "$DOMAIN" "$PLIST" 2>&1; then
  rm -f "$PLIST"
  restore_old_service
  vk_die "launchctl bootstrap 失败，已回滚。
  常见原因：同名服务还在 $DOMAIN 里没退干净。手工执行
    launchctl bootout $DOMAIN/$VK_LABEL
  再重跑本脚本。"
fi
launchctl enable "$DOMAIN/$VK_LABEL" 2>/dev/null || true
vk_ok "服务已加载（RunAtLoad + KeepAlive，开机自启）"

# 等它真的把端口起起来，别一装完就说成功。
started=0
for _ in $(seq 1 30); do
  if vk_port_in_use "$PORT"; then started=1; break; fi
  sleep 0.5
done

echo
launchctl print "$DOMAIN/$VK_LABEL" 2>/dev/null | sed -n '1,20p' || true
echo

if [ "$started" -eq 1 ]; then
  vk_ok "服务已在 $HOST:$PORT 监听"
else
  vk_warn "15 秒内没等到 $PORT 端口 LISTEN。看日志：
    tail -n 50 '$DATA_DIR/logs/stderr.log'"
fi

cat <<EOF

数据目录：$DATA_DIR
日志：    $DATA_DIR/logs/{stdout,stderr}.log
常用命令：
  停止    launchctl bootout $DOMAIN/$VK_LABEL
  重启    launchctl kickstart -k $DOMAIN/$VK_LABEL
  卸载    $SCRIPT_DIR/install-macos.sh --uninstall
EOF

if [ "$MODE" = team ]; then
  cat <<EOF

团队模式首次启动：库里还没有管理员时，服务会在日志里打印一次性初始化链接
（30 分钟过期，用一次作废）。用这条命令捞出来：
  grep -i setup '$DATA_DIR/logs/stdout.log' | tail -n 5
EOF
fi
