#!/usr/bin/env bash
# deploy/ 下各脚本共用的小工具。**被 source，不要直接执行。**
#
# 刻意不 `set -e`：调用方自己设。这里只提供函数。

VK_LABEL="ai.bloop.vibe-kanban"
VK_UNIT="vibe-kanban"

# ---- 输出 ------------------------------------------------------------------

vk_info() { printf '\033[0;36m==>\033[0m %s\n' "$*"; }
vk_ok()   { printf '\033[0;32m ✓ \033[0m%s\n' "$*"; }
vk_warn() { printf '\033[0;33m ! \033[0m%s\n' "$*" >&2; }
vk_die()  { printf '\033[0;31m ✗ \033[0m%s\n' "$*" >&2; exit 1; }

# ---- 数据目录 --------------------------------------------------------------

# 数据目录。与 crates/utils/src/assets.rs 的 prod_asset_dir_path() 一致：
#   macOS → ~/Library/Application Support/ai.bloop.vibe-kanban
#   Linux → ${XDG_DATA_HOME:-~/.local/share}/vibe-kanban
# 这两条路径写死在 Rust 侧（directories crate），改这里必须同步改那边。
vk_default_data_dir() {
  case "$(uname -s)" in
    Darwin) printf '%s/Library/Application Support/%s\n' "$HOME" "$VK_LABEL" ;;
    Linux)  printf '%s/%s\n' "${XDG_DATA_HOME:-$HOME/.local/share}" "$VK_UNIT" ;;
    *)      return 1 ;;
  esac
}

# 解析数据目录：$1（显式值）> $VK_DATA_DIR > 按平台推导。
vk_resolve_data_dir() {
  local explicit="${1:-}"
  if [ -n "$explicit" ]; then printf '%s\n' "$explicit"; return 0; fi
  if [ -n "${VK_DATA_DIR:-}" ]; then printf '%s\n' "$VK_DATA_DIR"; return 0; fi
  vk_default_data_dir || vk_die "无法推导数据目录（未知平台 $(uname -s)），请用 --data-dir 或 VK_DATA_DIR 显式指定"
}

vk_db_path() { printf '%s/db.v2.sqlite\n' "$1"; }

# 数据目录必须存在、可读、可写。不存在时按 $2 决定是创建还是报错。
#   vk_require_data_dir <dir> create|exist
vk_require_data_dir() {
  local dir="$1" mode="${2:-exist}"
  if [ ! -e "$dir" ]; then
    if [ "$mode" = create ]; then
      mkdir -p "$dir" || vk_die "创建数据目录失败：${dir}（检查上级目录权限与磁盘空间）"
    else
      vk_die "数据目录不存在：$dir
  服务从没跑过？先启动一次让它自己建，或用 --data-dir 指到正确位置。"
    fi
  fi
  [ -d "$dir" ] || vk_die "数据目录不是目录：$dir"
  [ -r "$dir" ] || vk_die "数据目录不可读：${dir}（当前用户 $(id -un)，属主 $(vk_owner_of "$dir")）"
  [ -w "$dir" ] || vk_die "数据目录不可写：${dir}（当前用户 $(id -un)，属主 $(vk_owner_of "$dir")）
  别用 sudo 跑这个脚本——数据目录属于登录用户，用 sudo 会把新文件的属主搞成 root，
  下次服务以普通用户身份启动就写不进去了。正确做法是修属主：
    chown -R \"\$(id -un)\" '$dir'"
}

vk_owner_of() { ls -ld "$1" 2>/dev/null | awk '{print $3}'; }

# ---- 依赖与端口 ------------------------------------------------------------

vk_need_cmd() {
  command -v "$1" >/dev/null 2>&1 || vk_die "缺少命令 $1。${2:-}"
}

# 端口是否已被占用。0=占用，1=空闲。
# 依次尝试 lsof / ss，都没有就用 bash 的 /dev/tcp 探测 127.0.0.1。
vk_port_in_use() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"$port" -sTCP:LISTEN >/dev/null 2>&1 && return 0
    return 1
  fi
  if command -v ss >/dev/null 2>&1; then
    ss -ltnH "sport = :$port" 2>/dev/null | grep -q . && return 0
    return 1
  fi
  (exec 3<>"/dev/tcp/127.0.0.1/$port") >/dev/null 2>&1 && { exec 3<&- 3>&- ; return 0; }
  return 1
}

# 占用端口的进程描述，用于报错时告诉用户「是谁占着」。
vk_port_holder() {
  local port="$1"
  if command -v lsof >/dev/null 2>&1; then
    lsof -nP -iTCP:"$port" -sTCP:LISTEN -Fcn 2>/dev/null \
      | awk '/^c/{c=substr($0,2)} /^n/{print c" ("substr($0,2)")"}' | head -n1
  fi
}

# ---- 磁盘空间 --------------------------------------------------------------

# 目录所在文件系统的可用空间（KiB）。df -P 是 POSIX 输出格式，
# macOS 与 Linux 都支持，列位置稳定（第 4 列 = Available）。
vk_avail_kb() {
  df -Pk "$1" 2>/dev/null | awk 'NR==2 {print $4}'
}

vk_size_kb() {
  # du -sk 对文件和目录都work；不存在返回 0。
  [ -e "$1" ] || { echo 0; return 0; }
  du -sk "$1" 2>/dev/null | awk '{print $1}'
}

# vk_require_space <目标目录> <需要的 KiB> <说明>
vk_require_space() {
  local dir="$1" need="$2" what="${3:-操作}"
  local avail; avail="$(vk_avail_kb "$dir")"
  [ -n "$avail" ] || { vk_warn "无法读取 $dir 的可用空间，跳过空间检查"; return 0; }
  if [ "$avail" -lt "$need" ]; then
    vk_die "$dir 可用空间不足：需要约 $((need / 1024)) MiB，只剩 $((avail / 1024)) MiB。$what 已中止，未改动任何文件。"
  fi
}

# ---- sqlite ----------------------------------------------------------------

vk_need_sqlite() {
  vk_need_cmd sqlite3 "macOS 自带；Debian/Ubuntu 用 'apt-get install sqlite3'，RHEL 系用 'dnf install sqlite'。"
}

# 库文件能打开且 integrity_check 通过。$2 为 quick 时用 quick_check（快很多）。
vk_check_db() {
  local db="$1" kind="${2:-full}" pragma out
  [ -f "$db" ] || { vk_warn "库文件不存在：$db"; return 1; }
  if [ "$kind" = quick ]; then pragma="quick_check"; else pragma="integrity_check"; fi
  out="$(sqlite3 "$db" "PRAGMA $pragma;" 2>&1)" || {
    vk_warn "sqlite3 打不开 ${db}：$out"
    return 1
  }
  [ "$out" = "ok" ] || { vk_warn "$db 的 $pragma 不是 ok：$out"; return 1; }
  return 0
}

# ---- 服务是否在跑 ----------------------------------------------------------

# 端口文件（crates/utils/src/port_file.rs）。MCP 也靠它找端口。
vk_port_file() { printf '%s/vibe-kanban/vibe-kanban.port\n' "${TMPDIR:-/tmp}"; }

# 服务看起来在跑吗？0=在跑。判据有两条，任一命中即算在跑：
#   1. 服务管理器说它 running（launchd / systemd）
#   2. 端口文件里的端口有人 LISTEN
vk_service_running() {
  case "$(uname -s)" in
    Darwin)
      if launchctl print "gui/$(id -u)/$VK_LABEL" 2>/dev/null | grep -qE 'state = running'; then
        return 0
      fi
      ;;
    Linux)
      if command -v systemctl >/dev/null 2>&1 \
        && systemctl --user is-active --quiet "$VK_UNIT" 2>/dev/null; then
        return 0
      fi
      ;;
  esac
  local pf port
  pf="$(vk_port_file)"
  if [ -f "$pf" ]; then
    port="$(sed -n 's/.*"main_port"[^0-9]*\([0-9]\{1,5\}\).*/\1/p' "$pf" 2>/dev/null | head -n1)"
    if [ -n "$port" ] && vk_port_in_use "$port"; then return 0; fi
  fi
  return 1
}

vk_stop_hint() {
  case "$(uname -s)" in
    Darwin) printf 'launchctl bootout gui/%s/%s\n' "$(id -u)" "$VK_LABEL" ;;
    Linux)  printf 'systemctl --user stop %s\n' "$VK_UNIT" ;;
    *)      printf '（手工停掉 vibe-kanban 进程）\n' ;;
  esac
}

vk_start_hint() {
  case "$(uname -s)" in
    Darwin) printf 'launchctl kickstart -k gui/%s/%s\n' "$(id -u)" "$VK_LABEL" ;;
    Linux)  printf 'systemctl --user start %s\n' "$VK_UNIT" ;;
    *)      printf '（手工启动 vibe-kanban）\n' ;;
  esac
}
