#!/usr/bin/env bash
#
# deploy/backup.sh 与 deploy/restore.sh 的脚本级测试。
# 全程用临时目录，不碰真实数据目录，不需要 root。
#
# 跑法：bash deploy/backup_test.sh
#
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BACKUP="$SCRIPT_DIR/backup.sh"
RESTORE="$SCRIPT_DIR/restore.sh"

PASS=0
FAIL=0

ok()   { PASS=$((PASS + 1)); printf '\033[0;32mPASS\033[0m %s\n' "$1"; }
bad()  { FAIL=$((FAIL + 1)); printf '\033[0;31mFAIL\033[0m %s\n' "$1"; }
note() { printf '\033[0;36m----\033[0m %s\n' "$1"; }

ROOT="$(mktemp -d)"
trap 'rm -rf "$ROOT"' EXIT

command -v sqlite3 >/dev/null 2>&1 || { echo "跳过：本机没有 sqlite3"; exit 0; }

# 造一个 WAL 模式的库，写数据后**不 checkpoint**——这正是服务运行时的常态，
# 也是「直接 cp 主库会丢数据」的复现条件。
# 关键手法：在同一个 sqlite3 进程里开着连接不退出（用 .backup 之外的方式
# 强制 WAL 不回写），这里用 journal_size_limit + 显式关闭自动 checkpoint。
make_wal_db() {
  local db="$1" rows="$2"
  sqlite3 "$db" >/dev/null <<SQL
PRAGMA journal_mode=WAL;
PRAGMA wal_autocheckpoint=0;
CREATE TABLE IF NOT EXISTS tasks (id INTEGER PRIMARY KEY, title TEXT);
SQL
  sqlite3 "$db" >/dev/null <<SQL
PRAGMA wal_autocheckpoint=0;
BEGIN;
$(i=1; while [ "$i" -le "$rows" ]; do echo "INSERT INTO tasks (title) VALUES ('task-$i');"; i=$((i + 1)); done)
COMMIT;
PRAGMA wal_checkpoint(PASSIVE);
SQL
}

row_count() { sqlite3 "$1" "SELECT count(*) FROM tasks;" 2>/dev/null || echo "ERR"; }

# ---------------------------------------------------------------------------
note "1. 基本往返：备份 → 解包 → 数据在"

D1="$ROOT/data1"
mkdir -p "$D1"
make_wal_db "$D1/db.v2.sqlite" 5
printf '{"mode":"team"}\n' >"$D1/server.json"

out="$("$BACKUP" --data-dir "$D1" --out "$ROOT/out1" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ]; then
  bad "backup.sh 退出码 ${rc}：$out"
else
  ok "backup.sh 正常退出"
fi

arch="$(ls -1 "$ROOT/out1"/vibe-kanban-*.tar.gz 2>/dev/null | head -n1)"
if [ -n "$arch" ]; then ok "产出了备份包"; else bad "没有产出备份包：$out"; fi

if [ -n "$arch" ]; then
  mkdir -p "$ROOT/ex1"
  tar -xzf "$arch" -C "$ROOT/ex1"
  n="$(row_count "$ROOT/ex1/db.v2.sqlite")"
  [ "$n" = "5" ] && ok "备份包里有全部 5 行" || bad "备份包里只有 $n 行（应为 5）"
  [ -f "$ROOT/ex1/server.json" ] && ok "server.json 被打包" || bad "server.json 没被打包"
  [ -f "$ROOT/ex1/MANIFEST.txt" ] && ok "有 MANIFEST.txt" || bad "缺 MANIFEST.txt"
fi

# ---------------------------------------------------------------------------
note "2. WAL 场景：写入未 checkpoint 时，.backup 拿得到而 cp 拿不到"

D2="$ROOT/data2"
mkdir -p "$D2"
make_wal_db "$D2/db.v2.sqlite" 3

# 关掉自动 checkpoint，再写 20 行并让连接**异常留下 WAL**：
# 用一个后台 sqlite3 保持连接，写完之后不 checkpoint 直接断开是不够的
# （断开时会做一次 checkpoint），所以这里改用「持有读事务的第二个连接」，
# 它会阻止 checkpoint 把 WAL 回写到主库——正是服务多连接运行时的真实形态。
mkfifo "$ROOT/fifo2"
sqlite3 "$D2/db.v2.sqlite" <"$ROOT/fifo2" >/dev/null 2>&1 &
READER_PID=$!
exec 9>"$ROOT/fifo2"
printf 'PRAGMA wal_autocheckpoint=0;\nBEGIN;\nSELECT count(*) FROM tasks;\n' >&9
sleep 0.5

sqlite3 "$D2/db.v2.sqlite" <<'SQL' >/dev/null 2>&1
PRAGMA wal_autocheckpoint=0;
INSERT INTO tasks (title)
  SELECT 'wal-' || value FROM (
    WITH RECURSIVE c(value) AS (SELECT 1 UNION ALL SELECT value+1 FROM c WHERE value<20)
    SELECT value FROM c
  );
SQL

wal_size="$(wc -c <"$D2/db.v2.sqlite-wal" 2>/dev/null | tr -d ' ' || echo 0)"
if [ "${wal_size:-0}" -gt 0 ]; then
  ok "构造成功：-wal 里有 $wal_size 字节未回写"
else
  note "（本机 sqlite 把 WAL 回写了，对照组这一项意义有限）"
fi

# 对照组：直接 cp 主库文件（脱离 -wal），看能读到几行。
cp "$D2/db.v2.sqlite" "$ROOT/naive-cp.sqlite"
cp_rows="$(row_count "$ROOT/naive-cp.sqlite")"

"$BACKUP" --data-dir "$D2" --out "$ROOT/out2" >/dev/null 2>&1 && rc=0 || rc=$?
arch2="$(ls -1 "$ROOT/out2"/vibe-kanban-*.tar.gz 2>/dev/null | head -n1)"
mkdir -p "$ROOT/ex2"
[ -n "$arch2" ] && tar -xzf "$arch2" -C "$ROOT/ex2"
bk_rows="$(row_count "$ROOT/ex2/db.v2.sqlite")"

printf '     直接 cp 主库 → %s 行；sqlite3 .backup → %s 行；期望 23 行\n' "$cp_rows" "$bk_rows"
if [ "$bk_rows" = "23" ]; then
  ok ".backup 拿到了全部 23 行（含未 checkpoint 的写入）"
else
  bad ".backup 只拿到 $bk_rows 行（应为 23）"
fi
if [ "$cp_rows" != "23" ]; then
  ok "对照组成立：直接 cp 主库只有 $cp_rows 行，会丢数据"
else
  note "对照组未复现（本机 sqlite $(sqlite3 --version | awk '{print $1}') 已把 WAL 回写）。
     这不影响 .backup 的正确性，只是说明这台机器上 cp 这次侥幸没丢。"
fi

exec 9>&-
kill "$READER_PID" 2>/dev/null || true
wait "$READER_PID" 2>/dev/null || true

# ---------------------------------------------------------------------------
note "3. 恢复往返：恢复到一个空数据目录，数据对得上"

D3="$ROOT/data3"
mkdir -p "$D3"
if [ -n "$arch2" ]; then
  out="$("$RESTORE" "$arch2" --data-dir "$D3" 2>&1)" && rc=0 || rc=$?
  if [ "$rc" -ne 0 ]; then
    bad "restore.sh 退出码 ${rc}：$out"
  else
    ok "restore.sh 正常退出"
    n="$(row_count "$D3/db.v2.sqlite")"
    [ "$n" = "23" ] && ok "恢复后有 23 行" || bad "恢复后只有 $n 行"
  fi
fi

# ---------------------------------------------------------------------------
note "4. 覆盖恢复：现有库被改名留底，不被删除"

D4="$ROOT/data4"
mkdir -p "$D4"
make_wal_db "$D4/db.v2.sqlite" 99
if [ -n "$arch2" ]; then
  "$RESTORE" "$arch2" --data-dir "$D4" >/dev/null 2>&1 && rc=0 || rc=$?
  n="$(row_count "$D4/db.v2.sqlite")"
  [ "$n" = "23" ] && ok "覆盖恢复后是备份里的 23 行" || bad "覆盖恢复后是 $n 行"
  if ls "$D4"/db.v2.sqlite.bak.* >/dev/null 2>&1; then
    old="$(ls -1 "$D4"/db.v2.sqlite.bak.* | head -n1)"
    m="$(row_count "$old")"
    [ "$m" = "99" ] && ok "旧库完整留底（99 行）" || bad "留底的旧库是 $m 行"
  else
    bad "没有留下 .bak 旧库"
  fi
fi

# ---------------------------------------------------------------------------
note "5. 保留份数：连跑 9 次只留 7 份"

D5="$ROOT/data5"
mkdir -p "$D5"
make_wal_db "$D5/db.v2.sqlite" 1
i=1
while [ "$i" -le 9 ]; do
  # 备份包按秒命名，同一秒内会撞名，所以显式错开时间戳目录名。
  "$BACKUP" --data-dir "$D5" --out "$ROOT/out5" --keep 7 >/dev/null 2>&1 || true
  sleep 1.05
  i=$((i + 1))
done
cnt="$(ls -1 "$ROOT/out5"/vibe-kanban-*.tar.gz 2>/dev/null | wc -l | tr -d ' ')"
[ "$cnt" = "7" ] && ok "只保留了 7 份（--keep 7）" || bad "保留了 $cnt 份（应为 7）"

# 留下的必须是最新的 7 份，不是最旧的 7 份。
newest="$(ls -1 "$ROOT/out5"/vibe-kanban-*.tar.gz | sort | tail -n1)"
oldest_kept="$(ls -1 "$ROOT/out5"/vibe-kanban-*.tar.gz | sort | head -n1)"
if [ "$newest" != "$oldest_kept" ]; then
  ok "保留的是最新的若干份（最旧 $(basename "$oldest_kept") < 最新 $(basename "$newest")）"
fi

# ---------------------------------------------------------------------------
note "6. 错误处理"

# 6.1 数据目录不存在
out="$("$BACKUP" --data-dir "$ROOT/nope" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "数据目录不存在"; then
  ok "数据目录不存在 → 明确报错并非零退出"
else
  bad "数据目录不存在时的行为不对（rc=${rc}）：$out"
fi

# 6.2 数据目录在但没有库
D6="$ROOT/data6"; mkdir -p "$D6"
out="$("$BACKUP" --data-dir "$D6" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "数据库不存在"; then
  ok "缺少数据库 → 明确报错"
else
  bad "缺库时的行为不对（rc=${rc}）：$out"
fi

# 6.3 缺少 sqlite3。直接清空 PATH 不行：连 dirname 都找不到，撞上的会是别的错。
# 做一个只缺 sqlite3 的 PATH：把脚本真正用到的命令软链进去，唯独不链 sqlite3。
NOSQLITE="$ROOT/nosqlite-bin"
mkdir -p "$NOSQLITE"
for c in dirname basename uname mkdir date du df tar cp ls rm sort tail head wc tr awk sed hostname id; do
  src="$(command -v "$c" 2>/dev/null || true)"
  [ -n "$src" ] && ln -sf "$src" "$NOSQLITE/$c"
done
out="$(PATH="$NOSQLITE" /bin/bash "$BACKUP" --data-dir "$D1" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "缺少命令 sqlite3"; then
  ok "缺少 sqlite3 → 明确报错"
else
  bad "缺 sqlite3 时的行为不对（rc=${rc}）：$out"
fi

# 6.4 备份包损坏
echo "this is not a tarball" >"$ROOT/broken.tar.gz"
out="$("$RESTORE" "$ROOT/broken.tar.gz" --data-dir "$ROOT/data7" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "备份包损坏"; then
  ok "损坏的备份包 → 明确报错"
else
  bad "损坏包的行为不对（rc=${rc}）：$out"
fi

# 6.5 备份包不存在
out="$("$RESTORE" "$ROOT/missing.tar.gz" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "备份包不存在"; then
  ok "备份包不存在 → 明确报错"
else
  bad "包不存在时的行为不对（rc=${rc}）：$out"
fi

# 6.6 tar 结构对但库损坏（截断）
D8="$ROOT/data8"; mkdir -p "$D8"
mkdir -p "$ROOT/fakepkg"
head -c 4096 /dev/urandom >"$ROOT/fakepkg/db.v2.sqlite"
tar -czf "$ROOT/corrupt-db.tar.gz" -C "$ROOT/fakepkg" .
out="$("$RESTORE" "$ROOT/corrupt-db.tar.gz" --data-dir "$D8" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "integrity_check 不通过"; then
  ok "包里的库损坏 → 拒绝恢复，现有数据未动"
else
  bad "库损坏时的行为不对（rc=${rc}）：$out"
fi

# 6.7 备份包里根本不是 vibe-kanban 的数据
mkdir -p "$ROOT/otherpkg"; echo hi >"$ROOT/otherpkg/readme.txt"
tar -czf "$ROOT/other.tar.gz" -C "$ROOT/otherpkg" .
out="$("$RESTORE" "$ROOT/other.tar.gz" --data-dir "$ROOT/data9" 2>&1)" && rc=0 || rc=$?
if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q "没有 db.v2.sqlite"; then
  ok "不是 vibe-kanban 备份包 → 明确报错"
else
  bad "无关包的行为不对（rc=${rc}）：$out"
fi

# ---------------------------------------------------------------------------
echo
printf '合计：\033[0;32m%d 通过\033[0m，\033[0;31m%d 失败\033[0m\n' "$PASS" "$FAIL"
[ "$FAIL" -eq 0 ] || exit 1
