#!/usr/bin/env bash
#
# 从 deploy/backup.sh 生成的备份包恢复数据目录。
#
# **顺序是「停服务 → 恢复 → 起服务」**，理由：
#   1. 服务持有 db.v2.sqlite 的连接和 -wal / -shm 两个伴生文件。在它跑着的时候
#      换掉主库，进程手里的旧 WAL 会被当成新库的 WAL 继续用，两边页号对不上，
#      结果是**新库当场被写坏**。
#   2. 恢复完再起服务，进程从干净状态重新打开库，WAL 从零开始。
# 所以本脚本检测到服务在跑会直接拒绝执行（--force 可跳过，但请先想清楚）。
#
# 恢复不删旧数据：现有的 db.v2.sqlite（含 -wal / -shm）会先改名成
# .bak.<时间戳> 留在原地，确认没问题之后自己删。
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/common.sh
. "$SCRIPT_DIR/common.sh"

ARCHIVE=""
DATA_DIR_ARG=""
FORCE=0
DB_ONLY=0

usage() {
  cat <<EOF
用法：$(basename "$0") <备份包.tar.gz> [选项]

选项：
  --data-dir PATH   恢复到哪个数据目录（默认按平台推导，或用 VK_DATA_DIR）
  --db-only         只恢复数据库，不覆盖 server.json / config.json / profiles.json
  --force           跳过「服务是否在跑」的检查（**不推荐**，见脚本头部注释）
  -h, --help        显示本帮助

标准流程：
  1) $(vk_stop_hint)
  2) $(basename "$0") <备份包.tar.gz>
  3) $(vk_start_hint)
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR_ARG="${2:?--data-dir 需要一个路径}"; shift 2 ;;
    --db-only)  DB_ONLY=1; shift ;;
    --force)    FORCE=1; shift ;;
    -h|--help)  usage; exit 0 ;;
    -*)         usage >&2; vk_die "未知参数：$1" ;;
    *)          ARCHIVE="$1"; shift ;;
  esac
done

[ -n "$ARCHIVE" ] || { usage >&2; vk_die "必须给出备份包路径"; }
[ -e "$ARCHIVE" ] || vk_die "备份包不存在：$ARCHIVE"
[ -f "$ARCHIVE" ] || vk_die "不是普通文件：$ARCHIVE"
[ -r "$ARCHIVE" ] || vk_die "备份包不可读：$ARCHIVE"
[ -s "$ARCHIVE" ] || vk_die "备份包是空文件：$ARCHIVE"

vk_need_sqlite

DATA_DIR="$(vk_resolve_data_dir "$DATA_DIR_ARG")"
vk_require_data_dir "$DATA_DIR" create

# ---- 服务必须先停 ----------------------------------------------------------

if vk_service_running; then
  if [ "$FORCE" -eq 0 ]; then
    vk_die "vibe-kanban 看起来还在运行，拒绝恢复。
  在服务持有数据库连接的情况下换掉库文件会当场写坏新库（见脚本头部注释）。
  先停服务：
    $(vk_stop_hint)
  恢复完再起：
    $(vk_start_hint)
  确实已经停了但检测误报，可以加 --force。"
  fi
  vk_warn "--force：跳过运行检查。如果服务真的在跑，恢复出来的库很可能是坏的。"
fi

# ---- 校验备份包 ------------------------------------------------------------

vk_info "校验备份包…"
tar -tzf "$ARCHIVE" >/dev/null 2>&1 \
  || vk_die "备份包损坏或不是 gzip 压缩的 tar：$ARCHIVE
  用 'tar -tzf \"$ARCHIVE\"' 看详细报错。换一份备份。"

WORK="$(mktemp -d)" || vk_die "无法创建临时目录"
trap 'rm -rf "$WORK"' EXIT

ARCHIVE_KB="$(vk_size_kb "$ARCHIVE")"
# gzip 压缩比按 1:8 估，再留一倍余量给「解包 + 拷进数据目录」两份。
vk_require_space "$WORK" "$(( ARCHIVE_KB * 16 + 20480 ))" "恢复"

tar -xzf "$ARCHIVE" -C "$WORK" || vk_die "解包失败：${ARCHIVE}（磁盘空间不足或文件损坏）"

NEW_DB="$WORK/db.v2.sqlite"
[ -f "$NEW_DB" ] || vk_die "备份包里没有 db.v2.sqlite，这不是 vibe-kanban 的备份包。
  包内容：$(tar -tzf "$ARCHIVE" 2>/dev/null | head -n 10 | tr '\n' ' ')"

vk_check_db "$NEW_DB" full \
  || vk_die "备份包里的数据库 integrity_check 不通过，拒绝恢复。
  现有数据一点没动。换一份更早的备份试试。"
vk_ok "备份包内的数据库通过 integrity_check"

if [ -f "$WORK/MANIFEST.txt" ]; then
  echo
  sed -n '1,8p' "$WORK/MANIFEST.txt"
  echo
fi

# ---- 空间检查 --------------------------------------------------------------

NEW_DB_KB="$(vk_size_kb "$NEW_DB")"
vk_require_space "$DATA_DIR" "$(( NEW_DB_KB + 20480 ))" "恢复"

# ---- 现有文件改名留底 ------------------------------------------------------

STAMP="$(date +%Y%m%d-%H%M%S)"
DB="$(vk_db_path "$DATA_DIR")"
SAVED=""

for suffix in "" "-wal" "-shm"; do
  src="$DB$suffix"
  if [ -e "$src" ]; then
    mv "$src" "$src.bak.$STAMP" || vk_die "改名 $src 失败（权限不足？）。恢复中止，现有数据未变。"
    SAVED="$SAVED $src.bak.$STAMP"
  fi
done
if [ -n "$SAVED" ]; then
  vk_ok "现有数据库已改名留底：$(echo "$SAVED" | tr ' ' '\n' | sed '/^$/d' | sed 's|.*/||' | tr '\n' ' ')"
fi

# ---- 拷回 ------------------------------------------------------------------

restore_failed() {
  vk_warn "恢复失败，正在回滚到原来的数据库…"
  rm -f "$DB"
  for suffix in "" "-wal" "-shm"; do
    if [ -e "$DB$suffix.bak.$STAMP" ]; then
      mv "$DB$suffix.bak.$STAMP" "$DB$suffix" || true
    fi
  done
  vk_die "$1"
}

cp "$NEW_DB" "$DB" || restore_failed "写入 $DB 失败（磁盘空间不足或权限问题）。已回滚。"
chmod 600 "$DB" 2>/dev/null || true

# 拷回来的库必须能打开。到这一步还失败就回滚，别留一个半截文件在那儿。
vk_check_db "$DB" full || restore_failed "拷回后的数据库校验失败。已回滚。"
vk_ok "数据库已恢复：$DB"

if [ "$DB_ONLY" -eq 0 ]; then
  for f in server.json config.json profiles.json; do
    if [ -f "$WORK/$f" ]; then
      if [ -f "$DATA_DIR/$f" ]; then
        cp "$DATA_DIR/$f" "$DATA_DIR/$f.bak.$STAMP" || true
      fi
      cp "$WORK/$f" "$DATA_DIR/$f" || vk_warn "恢复 $f 失败，跳过（数据库已成功恢复）"
      vk_ok "已恢复 $f"
    fi
  done
else
  vk_info "--db-only：跳过 server.json / config.json / profiles.json"
fi

cat <<EOF

恢复完成。

留底文件（确认一切正常后可以删）：
$(ls -1 "$DATA_DIR"/*.bak."$STAMP" 2>/dev/null | sed 's/^/  /' || echo "  （无）")

下一步：
  $(vk_start_hint)

注意：备份包**不含**本机凭据（credentials.json、machine_token、签名私钥），
所以这台机器可能需要重新登录云端账号、重新配对 MCP。这是刻意的——
本机凭据进备份包会把泄露面扩大到每一份备份。
EOF
