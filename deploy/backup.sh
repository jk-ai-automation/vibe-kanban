#!/usr/bin/env bash
#
# 备份 vibe-kanban 的数据目录。
#
# **为什么不能直接 cp 主库文件**：数据库跑在 WAL 模式下（crates/db/src/lib.rs），
# 最近的写入躺在 db.v2.sqlite-wal 里还没 checkpoint 回主库。单独 cp 主库文件
# 得到的是一个「少了最近一段时间全部写入」的旧快照，而且它能正常打开、
# integrity_check 也返回 ok——所以这种损坏在恢复之前完全无法察觉。
# 这里一律用 sqlite3 的 .backup（在线备份 API，自己处理 WAL 与并发写）。
#
# 服务**在跑**的时候也能安全备份，不需要停服务。
#
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=deploy/common.sh
. "$SCRIPT_DIR/common.sh"

# 会被打进备份包的配置文件。**刻意不含**本机凭据：
#   credentials.json / machine_token / server_ed25519_signing_key
#   trusted_ed25519_public_keys.json / relay_host_credentials.json
# 它们是这台机器的身份，进了备份包就等于把泄露面从「一台机器」扩大到
# 「每一份备份、每一个存备份的网盘」。恢复后重新登录云端 / 重新配对即可。
CONFIG_FILES="server.json config.json profiles.json"

DATA_DIR_ARG=""
OUT_DIR_ARG=""
KEEP="${VK_BACKUP_KEEP:-7}"

usage() {
  cat <<EOF
用法：$(basename "$0") [输出目录] [选项]

选项：
  --data-dir PATH   数据目录（默认按平台推导，或用 VK_DATA_DIR）
  --out PATH        输出目录（默认 <数据目录>/backups）
  --keep N          保留最近 N 份（默认 ${KEEP}，也可用 VK_BACKUP_KEEP）
  -h, --help        显示本帮助

备份包内容：
  db.v2.sqlite      用 sqlite3 .backup 生成的一致性快照
  $CONFIG_FILES
  MANIFEST.txt      时间、主机、来源路径、文件清单

**不含**本机凭据（credentials.json、machine_token、签名私钥等）。
恢复到新机器后需要重新登录云端账号。
EOF
}

while [ $# -gt 0 ]; do
  case "$1" in
    --data-dir) DATA_DIR_ARG="${2:?--data-dir 需要一个路径}"; shift 2 ;;
    --out)      OUT_DIR_ARG="${2:?--out 需要一个路径}"; shift 2 ;;
    --keep)     KEEP="${2:?--keep 需要一个数字}"; shift 2 ;;
    -h|--help)  usage; exit 0 ;;
    -*)         usage >&2; vk_die "未知参数：$1" ;;
    *)          OUT_DIR_ARG="$1"; shift ;;
  esac
done

case "$KEEP" in
  ''|*[!0-9]*) vk_die "--keep 必须是非负整数，收到：$KEEP" ;;
esac

vk_need_sqlite

DATA_DIR="$(vk_resolve_data_dir "$DATA_DIR_ARG")"
vk_require_data_dir "$DATA_DIR" exist

DB="$(vk_db_path "$DATA_DIR")"
[ -f "$DB" ] || vk_die "数据库不存在：$DB
  数据目录对不对？服务至少要成功启动过一次才会建库。"
[ -r "$DB" ] || vk_die "数据库不可读：${DB}（当前用户 $(id -un)，属主 $(vk_owner_of "$DB")）"

OUT_DIR="${OUT_DIR_ARG:-$DATA_DIR/backups}"
mkdir -p "$OUT_DIR" || vk_die "创建输出目录失败：$OUT_DIR"
[ -w "$OUT_DIR" ] || vk_die "输出目录不可写：$OUT_DIR"

# 空间预算：备份库（≈主库+WAL）+ tar.gz 各一份，再加 20 MiB 余量。
# 空间不够时 sqlite3 .backup 会写出一个截断的库文件，必须提前拦住。
DB_KB="$(vk_size_kb "$DB")"
WAL_KB="$(vk_size_kb "$DB-wal")"
NEED_KB=$(( DB_KB + WAL_KB ))
NEED_KB=$(( NEED_KB * 2 + 20480 ))
vk_require_space "$OUT_DIR" "$NEED_KB" "备份"

# 源库先自检。备份一个已经坏掉的库只会把坏数据固化成「最新的好备份」。
if ! vk_check_db "$DB" quick; then
  vk_die "源数据库 quick_check 不通过：$DB
  这份库已经有问题，先修复或从上一个备份恢复，不要在它之上再做备份。"
fi

STAMP="$(date +%Y%m%d-%H%M%S)"
ARCHIVE="$OUT_DIR/vibe-kanban-$STAMP.tar.gz"
if [ -e "$ARCHIVE" ]; then
  vk_die "备份文件已存在：${ARCHIVE}（同一秒内跑了两次？）"
fi

WORK="$(mktemp -d)" || vk_die "无法创建临时目录"
trap 'rm -rf "$WORK"' EXIT
vk_require_space "$WORK" "$(( DB_KB + WAL_KB + 20480 ))" "备份"

vk_info "备份数据库（sqlite3 .backup，非 cp）…"
# 在 $WORK 里用相对文件名，避开「路径含单引号会把 .backup 的参数截断」的坑。
if ! ( cd "$WORK" && sqlite3 "$DB" ".backup 'db.v2.sqlite'" ); then
  vk_die "sqlite3 .backup 失败。常见原因：磁盘满、库文件被独占锁住、权限不足。
  未产出备份文件。"
fi
[ -s "$WORK/db.v2.sqlite" ] || vk_die "sqlite3 .backup 没有产出内容（磁盘满？）。未产出备份文件。"

# 备份产物自检：这是「备份可用」与「备份只是存在」之间唯一的区别。
vk_check_db "$WORK/db.v2.sqlite" full \
  || vk_die "备份出来的库 integrity_check 不通过，已丢弃，未产出备份文件。"
vk_ok "数据库快照通过 integrity_check"

for f in $CONFIG_FILES; do
  if [ -f "$DATA_DIR/$f" ]; then
    cp "$DATA_DIR/$f" "$WORK/$f" || vk_die "复制 $f 失败"
  fi
done

{
  echo "vibe-kanban backup"
  echo "created_at: $(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "host: $(hostname 2>/dev/null || echo unknown)"
  echo "source_data_dir: $DATA_DIR"
  echo "sqlite3: $(sqlite3 --version 2>/dev/null | awk '{print $1}')"
  echo "method: sqlite3 .backup (WAL-safe)"
  echo "excludes: credentials.json machine_token server_ed25519_signing_key trusted_ed25519_public_keys.json relay_host_credentials.json"
  echo "files:"
  ( cd "$WORK" && ls -1 )
} >"$WORK/MANIFEST.txt"

vk_info "打包…"
tar -czf "$ARCHIVE" -C "$WORK" . || {
  rm -f "$ARCHIVE"
  vk_die "打包失败，已清理半成品。"
}

# 立刻回读一次：写坏的 tar 现在发现，比恢复那天发现好。
tar -tzf "$ARCHIVE" >/dev/null 2>&1 || {
  rm -f "$ARCHIVE"
  vk_die "生成的备份包读不回来，已删除。磁盘可能有问题。"
}

vk_ok "备份完成：${ARCHIVE}（$(du -h "$ARCHIVE" 2>/dev/null | awk '{print $1}')）"

# ---- 保留最近 N 份 ---------------------------------------------------------

if [ "$KEEP" -gt 0 ]; then
  # 不用 `ls | xargs -r`：BSD xargs 没有 -r，空输入时会把 rm 跑成 `rm -f` 之外
  # 的形态。按文件名排序而不是 mtime——文件名里的时间戳才是备份时刻的真相。
  removed=0
  while IFS= read -r old; do
    [ -n "$old" ] || continue
    rm -f "$old" && removed=$((removed + 1))
  done <<EOF
$(ls -1 "$OUT_DIR"/vibe-kanban-*.tar.gz 2>/dev/null | sort -r | tail -n +"$((KEEP + 1))")
EOF
  if [ "$removed" -gt 0 ]; then
    vk_ok "已按 --keep $KEEP 清理 $removed 份旧备份"
  fi
fi

remaining="$(ls -1 "$OUT_DIR"/vibe-kanban-*.tar.gz 2>/dev/null | wc -l | tr -d ' ')"
echo "目录 $OUT_DIR 现有 $remaining 份备份。"
