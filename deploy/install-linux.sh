#!/usr/bin/env bash
#
# 把 vibe-kanban 装成 systemd **用户** unit。
#
# 幂等：重复跑会覆盖 unit 文件并 restart，不会留下两份。
# 可回滚：`./install-linux.sh --uninstall` 卸干净（**不删数据目录**）。
# 不用 sudo：装到 ~/.config/systemd/user/，全程不碰系统目录。
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/common.sh
. "$SCRIPT_DIR/common.sh"

TEMPLATE="$SCRIPT_DIR/systemd/$VK_UNIT.service"
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT_PATH="$UNIT_DIR/$VK_UNIT.service"

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
  --data-dir PATH   数据目录                   默认 \${XDG_DATA_HOME:-~/.local/share}/$VK_UNIT
  --dry-run         只打印将要写入的 unit，不动系统
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

[ "$(uname -s)" = "Linux" ] || vk_die "这个脚本只用于 Linux。macOS 请用 deploy/install-macos.sh。"
[ "$(id -u)" != "0" ] || vk_die "不要用 sudo / root 跑这个脚本。
  它装的是当前登录用户的 systemd user unit，root 跑出来的服务读不到你的 ~/.ssh 与
  各家编码代理 CLI 的登录态，数据目录的属主也会变成 root。"

vk_need_cmd systemctl "这台机器没有 systemd。可以改用 supervisor / 直接跑二进制，
  或参考 deploy/systemd/$VK_UNIT.service 里的环境变量自己写启动脚本。"

if ! systemctl --user show-environment >/dev/null 2>&1; then
  vk_die "连不上 systemd 用户实例（systemctl --user 不可用）。
  常见于 SSH 进来的会话没有 user D-Bus。先试：
    export XDG_RUNTIME_DIR=/run/user/\$(id -u)
    loginctl enable-linger \$(id -un)
  然后重新登录再跑本脚本。"
fi

# ---- 卸载 ------------------------------------------------------------------

uninstall() {
  local found=0
  if systemctl --user list-unit-files "$VK_UNIT.service" 2>/dev/null | grep -q "$VK_UNIT"; then
    found=1
  fi
  if [ -f "$UNIT_PATH" ]; then found=1; fi
  if [ "$found" -eq 0 ]; then
    vk_warn "服务未安装：既没有 $VK_UNIT.service，也没有 ${UNIT_PATH}。无需卸载。"
    return 0
  fi
  vk_info "停止并禁用服务…"
  systemctl --user disable --now "$VK_UNIT" 2>/dev/null || true
  rm -f "$UNIT_PATH"
  systemctl --user daemon-reload 2>/dev/null || true
  systemctl --user reset-failed "$VK_UNIT" 2>/dev/null || true
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
[ "$PORT" -ge 1024 ] || vk_die "--port $PORT 是特权端口，用户 unit 绑不上。请用 >=1024 的端口，
  真要用 80/443 就在前面放一层反向代理（见 deploy/Caddyfile.selfhost.example）。"

if [ -z "$HOST" ]; then
  if [ "$MODE" = team ]; then HOST="0.0.0.0"; else HOST="127.0.0.1"; fi
fi

# 个人模式免登录：谁能连上谁就有全部权限（能建工作区、跑命令、读仓库）。
if [ "$MODE" = personal ] && [ "$HOST" != "127.0.0.1" ] && [ "$HOST" != "localhost" ] && [ "$HOST" != "::1" ]; then
  vk_die "拒绝把 personal 模式监听在 ${HOST}。
  个人模式没有登录，网络可达即全权——同网段任何人都能建工作区、在你的机器上跑命令。
  要多人用请改成 --mode team（强制登录），要本机用请去掉 --host。"
fi

# 二进制必须存在且可执行。少了这一条，systemd 会以 status=203/EXEC 无限重启。
[ -e "$BIN" ] || vk_die "可执行文件不存在：$BIN"
BIN="$(cd "$(dirname "$BIN")" && pwd)/$(basename "$BIN")"
[ -f "$BIN" ] || vk_die "--bin 不是普通文件：$BIN"
[ -x "$BIN" ] || vk_die "可执行文件没有执行权限：$BIN
  修复：chmod +x '$BIN'"

DATA_DIR="$(vk_resolve_data_dir "$DATA_DIR_ARG")"
vk_require_data_dir "$DATA_DIR" create
mkdir -p "$DATA_DIR/logs" || vk_die "创建日志目录失败：$DATA_DIR/logs"
vk_require_space "$DATA_DIR" 51200 "安装"

[ -f "$TEMPLATE" ] || vk_die "找不到 unit 模板：${TEMPLATE}（部署包不完整？）"
mkdir -p "$UNIT_DIR" || vk_die "创建 $UNIT_DIR 失败"

# ---- 渲染 unit -------------------------------------------------------------

# systemd 的 Environment= 值里如果有空格必须加引号；路径里有 % 要写成 %%。
unit_escape() { printf '%s' "$1" | sed -e 's/%/%%/g'; }

RENDERED="$(mktemp)" || vk_die "无法创建临时文件"
trap 'rm -f "$RENDERED"' EXIT

# 先砍掉 [Unit] 之前的说明性注释：它里面也有 __占位符__ 字样，留着会让
# 下面的「还有没替换的占位符」检查误报。
sed \
  -e "/^\[Unit\]/,\$!d" \
  -e "s|__BIN_PATH__|$(unit_escape "$BIN")|g" \
  -e "s|__DATA_DIR__|$(unit_escape "$DATA_DIR")|g" \
  -e "s|__MODE__|$(unit_escape "$MODE")|g" \
  -e "s|__HOST__|$(unit_escape "$HOST")|g" \
  -e "s|__PORT__|$(unit_escape "$PORT")|g" \
  -e "s|__ALLOWED_ORIGINS__|$(unit_escape "$ORIGINS")|g" \
  -e "s|__PATH__|$(unit_escape "$PATH")|g" \
  "$TEMPLATE" >"$RENDERED"

if grep -q '__[A-Z_]*__' "$RENDERED"; then
  vk_die "unit 里还有没替换的占位符：$(grep -o '__[A-Z_]*__' "$RENDERED" | sort -u | tr '\n' ' ')"
fi

# StandardOutput=append: 需要 systemd >= 240。老版本上这一行会让 unit 起不来，
# 所以在这里就降级成 journal，而不是等用户去猜。
SYSTEMD_VER="$(systemctl --version 2>/dev/null | awk 'NR==1 {print $2}')"
case "$SYSTEMD_VER" in
  ''|*[!0-9]*) vk_warn "读不出 systemd 版本，保留 append: 日志配置" ;;
  *)
    if [ "$SYSTEMD_VER" -lt 240 ]; then
      vk_warn "systemd $SYSTEMD_VER < 240，不支持 append: 日志；改用 journal。
  看日志请用：journalctl --user -u $VK_UNIT -f"
      sed -i -e "s|^StandardOutput=append:.*|StandardOutput=journal|" \
             -e "s|^StandardError=append:.*|StandardError=journal|" "$RENDERED"
    fi
    ;;
esac

if [ "$DRY_RUN" -eq 1 ]; then
  vk_info "--dry-run：以下内容将写入 $UNIT_PATH"
  cat "$RENDERED"
  exit 0
fi

# ---- 先停旧的，再查端口 ----------------------------------------------------

# 顺序很重要：先停我们自己的旧实例再查端口，否则重装永远被自己挡住。
if systemctl --user is-active --quiet "$VK_UNIT" 2>/dev/null; then
  vk_info "检测到运行中的服务，先停掉（重装）…"
  systemctl --user stop "$VK_UNIT" || true
  for _ in 1 2 3 4 5 6 7 8 9 10; do
    vk_port_in_use "$PORT" || break
    sleep 0.3
  done
fi

if vk_port_in_use "$PORT"; then
  holder="$(vk_port_holder "$PORT")"
  vk_die "端口 $PORT 已被占用${holder:+（${holder}）}。
  换一个端口（--port）或先停掉占用方。服务未安装，系统未被改动。"
fi

# ---- 安装并启动 ------------------------------------------------------------

HAD_UNIT=0
if [ -f "$UNIT_PATH" ]; then HAD_UNIT=1; fi
BACKUP=""
if [ "$HAD_UNIT" -eq 1 ]; then
  BACKUP="$UNIT_PATH.bak.$(date +%Y%m%d-%H%M%S)"
  cp "$UNIT_PATH" "$BACKUP"
fi

install -m 644 "$RENDERED" "$UNIT_PATH" || vk_die "写入 $UNIT_PATH 失败"
vk_ok "已写入 $UNIT_PATH"

systemctl --user daemon-reload || vk_die "systemctl --user daemon-reload 失败"

if ! systemctl --user enable --now "$VK_UNIT"; then
  # 回滚到安装前的状态，别留一个起不来的 unit 在那儿自启。
  if [ "$HAD_UNIT" -eq 1 ] && [ -n "$BACKUP" ]; then
    mv "$BACKUP" "$UNIT_PATH"
  else
    rm -f "$UNIT_PATH"
  fi
  systemctl --user daemon-reload || true
  vk_die "启动失败，已回滚 unit 文件。看原因：
    systemctl --user status $VK_UNIT
    journalctl --user -u $VK_UNIT -n 50"
fi
if [ -n "$BACKUP" ]; then rm -f "$BACKUP"; fi
vk_ok "服务已启用并启动（Restart=always，登录即自启）"

started=0
for _ in $(seq 1 30); do
  if vk_port_in_use "$PORT"; then started=1; break; fi
  sleep 0.5
done

echo
systemctl --user --no-pager status "$VK_UNIT" 2>/dev/null | sed -n '1,15p' || true
echo

if [ "$started" -eq 1 ]; then
  vk_ok "服务已在 $HOST:$PORT 监听"
else
  vk_warn "15 秒内没等到 $PORT 端口 LISTEN。看日志：
    journalctl --user -u $VK_UNIT -n 50"
fi

# linger：不开的话，用户一注销 systemd 用户实例就被回收，服务跟着停。
# 对一台「没人坐在前面」的局域网服务器来说这基本是必开项。
if command -v loginctl >/dev/null 2>&1; then
  if [ "$(loginctl show-user "$(id -un)" -p Linger --value 2>/dev/null || echo no)" != "yes" ]; then
    vk_warn "未开启 linger：你一注销，服务就会停。开启（需要一次 sudo）：
    sudo loginctl enable-linger $(id -un)"
  fi
fi

cat <<EOF

数据目录：$DATA_DIR
日志：    $DATA_DIR/logs/{stdout,stderr}.log（或 journalctl --user -u ${VK_UNIT}）
常用命令：
  停止    systemctl --user stop $VK_UNIT
  重启    systemctl --user restart $VK_UNIT
  卸载    $SCRIPT_DIR/install-linux.sh --uninstall
EOF

if [ "$MODE" = team ]; then
  cat <<EOF

团队模式首次启动：库里还没有管理员时，服务会在日志里打印一次性初始化链接
（30 分钟过期，用一次作废）。用这条命令捞出来：
  grep -i setup '$DATA_DIR/logs/stdout.log' | tail -n 5
EOF
fi
