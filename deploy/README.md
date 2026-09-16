# 自建部署

一个可执行文件 + 一个数据目录。不需要 Docker，不需要 Postgres，不需要 root。

- [数据目录](#数据目录)
- [两种运行模式](#两种运行模式)
- [装成开机自启的服务](#装成开机自启的服务)
- [配置文件 server.json](#配置文件-serverjson)
- [局域网访问](#局域网访问)
- [HTTPS](#https)
- [第三方登录](#第三方登录)
- [备份与恢复](#备份与恢复)
- [排查](#排查)

---

## 数据目录

所有状态都在一个目录里：

| 平台 | 路径 |
|---|---|
| macOS | `~/Library/Application Support/ai.bloop.vibe-kanban` |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/vibe-kanban` |

```
db.v2.sqlite        全部业务数据（WAL 模式）
db.v2.sqlite-wal    ┐ 主库的伴生文件，三个一起才是完整数据库
db.v2.sqlite-shm    ┘
server.json         运行模式、会话、OAuth 凭据（可选，不存在就全走默认值）
config.json         应用配置
profiles.json       编码代理配置
credentials.json    云端登录态          ← 本机凭据，不进备份包
machine_token       MCP 用的本机令牌     ← 同上，权限 0600
logs/               服务化安装后的 stdout / stderr
backups/            backup.sh 的默认输出目录
```

**备份时绝不能只拷 `db.v2.sqlite`** —— 见[备份与恢复](#备份与恢复)。

---

## 两种运行模式

| | `VK_MODE=personal`（默认） | `VK_MODE=team` |
|---|---|---|
| 登录 | 无。中间件注入本机用户 | 强制。未登录访问 `/api/*` 一律 401 |
| 建议监听 | `127.0.0.1` | `0.0.0.0`（配合登录） |
| 适用 | 自己一台机器 | 团队共用一台机器 |
| 云端 relay | 支持 | **不支持**，团队模式下 relay 请求一律 401 |

```bash
# 个人版：直接跑
./vibe-kanban

# 团队版：监听全网卡
VK_MODE=team HOST=0.0.0.0 BACKEND_PORT=8080 ./vibe-kanban
```

首次以 `team` 模式启动、库里还没有能登录的管理员时，服务会在控制台打印一条
**一次性初始化链接**（含令牌，30 分钟过期，用一次作废，重启重新生成）。
浏览器打开它创建第一个管理员账号。

> **个人模式不要监听 `0.0.0.0`。** 个人模式没有登录，网络可达即全权：同网段任何
> 人都能建工作区、在这台机器上跑命令、读你所有仓库。需要多人用就切
> `VK_MODE=team`。安装脚本会直接拒绝 `--mode personal --host 0.0.0.0` 这种组合。

### 环境变量

| 变量 | 默认 | 说明 |
|---|---|---|
| `VK_MODE` | `personal` | `personal` / `team` |
| `HOST` | `127.0.0.1` | 监听地址 |
| `BACKEND_PORT`（或 `PORT`） | 随机空闲端口 | 监听端口 |
| `VK_ALLOWED_ORIGINS` | 空 | 额外放行的来源，逗号分隔。见[局域网访问](#局域网访问) |
| `VK_PUBLIC_BASE_URL` | 空 | 成员实际访问的基址。**决定 Cookie 带不带 `Secure`**，也决定 OAuth 回调 |
| `VK_TRUST_PROXY` | `0` | 采信 `X-Forwarded-For` 取客户端 IP。**只在反代后面开** |
| `VK_SESSION_TTL_DAYS` | `30` | 会话有效期，1–3650 |
| `VK_ALLOW_OAUTH_SIGNUP` | `0` | 允许第三方登录自动建号 |
| `VK_SQLITE_WAL` | `1` | 设 `0` 回退到非 WAL journal 模式 |
| `VK_OAUTH_{FEISHU,LARK,GOOGLE}_CLIENT_{ID,SECRET}` | 空 | OAuth 凭据，覆盖 `server.json` |

开关类变量只认 `1`/`true` 为真、`0`/`false` 为假，其它写法一律回落到默认值——
这是刻意的，拼错不会意外打开一个防护开关。

**环境变量优先级高于 `server.json`。**

---

## 装成开机自启的服务

```bash
# macOS（launchd 用户代理）
./deploy/install-macos.sh --bin "$PWD/vibe-kanban" --port 8080

# Linux（systemd 用户 unit）
./deploy/install-linux.sh --bin "$PWD/vibe-kanban" --port 8080
```

两个脚本都：

- 默认 `--mode team --host 0.0.0.0`，`Restart=always` 等价语义，日志落到
  `<数据目录>/logs/`
- **不用 sudo**，检测到 root 会直接拒绝（root 跑出来的服务读不到你的 `~/.ssh`
  和各家编码代理 CLI 的登录态，数据目录属主也会变成 root）
- 幂等：重复跑先停旧实例再装新的；装之前检查二进制存在且可执行、端口没被占、
  数据目录可写；中途失败会把之前在跑的实例原样恢复回去
- `--dry-run` 只打印渲染后的 plist / unit，不动系统
- `--uninstall` 卸干净，**不删数据目录**

常用选项：`--mode` `--host` `--port` `--origins` `--data-dir`。

### Linux 额外一步

systemd **用户** unit 在你注销后会被回收。一台没人坐在前面的服务器上必须开
linger：

```bash
sudo loginctl enable-linger "$(id -un)"
```

安装脚本检测到没开会提醒。

`PrivateTmp` 必须保持 `false`：MCP 客户端靠 `$TMPDIR/vibe-kanban/vibe-kanban.port`
找服务端口，开了 `PrivateTmp` 端口文件会落进私有命名空间，表现是「服务活着但
MCP 连不上」。

### 手工管理

| | macOS | Linux |
|---|---|---|
| 停 | `launchctl bootout gui/$(id -u)/ai.bloop.vibe-kanban` | `systemctl --user stop vibe-kanban` |
| 起 | `launchctl kickstart -k gui/$(id -u)/ai.bloop.vibe-kanban` | `systemctl --user start vibe-kanban` |
| 日志 | `tail -f <数据目录>/logs/stderr.log` | 同左，或 `journalctl --user -u vibe-kanban -f` |

---

## 配置文件 server.json

放在数据目录下，全部字段可选。样例见 `deploy/server.example.json`。

```json
{
  "mode": "team",
  "allow_oauth_signup": false,
  "session_ttl_days": 30,
  "sqlite_wal": true,
  "trust_proxy": false,
  "public_base_url": "https://kanban.example.com",
  "oauth": {
    "feishu": { "client_id": "", "client_secret": "" },
    "lark":   { "client_id": "", "client_secret": "" },
    "google": { "client_id": "", "client_secret": "" }
  }
}
```

一个提供方**必须** `client_id` 与 `client_secret` 都非空才算配好；只配一半的
提供方不会出现在登录页（否则就是一个点下去必然失败的按钮）。未列在
`feishu` / `lark` / `google` 里的 key 一律忽略。

**文件写坏会让服务启动失败，而不是静默退回个人版**——退回免登录属于 fail-open，
所以刻意不这么做。

---

## 局域网访问

最简单的形态：团队模式 + 监听 `0.0.0.0` + 成员用 `http://<服务器IP>:8080` 访问。

### `VK_ALLOWED_ORIGINS` 什么时候需要配

来源校验的实际规则是：**`Origin` 与 `Host` 对得上就放行**。成员直接用
`http://<服务器IP>:8080` 访问时两者天然一致，**不配 `VK_ALLOWED_ORIGINS` 也能用**。

需要显式配置的情况：

- 反向代理**改写了 `Host` 头**（例如 Caddy 里写了 `header_up Host 127.0.0.1:8080`）
- 成员通过与后端看到的 `Host` 不同的别名访问（DNS 别名、不同端口的转发）

配置格式是完整来源，逗号分隔，**不要带路径**：

```bash
VK_ALLOWED_ORIGINS="http://kanban.lan:8080,https://kanban.example.com"
```

### 明文 HTTP 的限制

| 事实 | 后果 |
|---|---|
| 没配 `VK_PUBLIC_BASE_URL`（或它不是 `https://`）时，会话 Cookie **不带 `Secure`** | Cookie 会在明文链路上传输，同网段可被嗅探 |
| Cookie 是 `SameSite=Lax` + `HttpOnly`，写请求另有 Origin 强校验与双提交令牌 | CSRF 有防护，但防不了链路嗅探 |
| 密码在明文 HTTP 上是明文传输 | 同上 |

**所以明文 HTTP 只建议在受信任的内网使用，绝不要暴露到公网。** 需要在不受信任
的网络里用，就上 [HTTPS](#https)。

反过来也要注意：**明文访问时不要配 `VK_PUBLIC_BASE_URL=https://...`**。那会让服务
发出带 `Secure` 的 Cookie，浏览器在 HTTP 页面上直接丢弃它，表现是「登录成功但
立刻弹回登录页」。

---

## HTTPS

用 Caddy 反代，自动申请并续期证书。完整样例见
`deploy/Caddyfile.selfhost.example`：

```caddy
kanban.example.com {
    encode gzip
    reverse_proxy 127.0.0.1:8080 {
        header_up X-Forwarded-For {remote_host}
        header_up X-Forwarded-Proto {scheme}
        flush_interval -1
    }
}
```

后端相应地改成：

```bash
VK_MODE=team
HOST=127.0.0.1                                   # 只监听回环
BACKEND_PORT=8080
VK_TRUST_PROXY=1
VK_PUBLIC_BASE_URL=https://kanban.example.com
VK_ALLOWED_ORIGINS=https://kanban.example.com
```

三点必须说清楚：

1. **`VK_TRUST_PROXY=1` 是上了反代之后的必选项。** 不开的话，所有请求在后端看来
   都来自代理那一个 IP，登录限速会按这个 IP 分桶——一个人输错几次密码，全公司
   都被锁在外面。

2. **反代必须*覆写* `X-Forwarded-For`，不能追加。** `VK_TRUST_PROXY=1` 打开后，
   后端**无条件**采信 `X-Forwarded-For` 的第一段，没有上游 IP 白名单。如果反代
   用的是追加模式（`header_up +X-Forwarded-For`），攻击者自带一个
   `X-Forwarded-For: 1.2.3.4` 就能让每次登录尝试落进不同的限速桶，暴力破解防护
   直接失效。上面样例里的 `header_up X-Forwarded-For {remote_host}` 是**整个替换**，
   这是正确写法。

3. **`HOST` 要改回 `127.0.0.1`。** 上了反代还监听 `0.0.0.0`，同网段的人可以绕过
   Caddy 直连 8080——明文，而且这时候 `VK_TRUST_PROXY=1` 会让他们自带的
   `X-Forwarded-For` 被原样采信，限速形同虚设。

内网域名拿不到公网证书时，可以用 Caddy 的 `tls internal` 自签，代价是每台客户端
都要手工信任 Caddy 的根证书。

---

## 第三方登录

支持飞书、Lark、Google。凭据写进 `server.json` 的 `oauth` 段或对应环境变量。

> **Google 登录在 `http://<IP>:<端口>` 下不可用。** Google 不接受裸 IP 作为
> OAuth 回调地址（也不接受 `http://` 的非 localhost 回调）。要用 Google 登录，
> 必须有域名 + HTTPS，也就是走上面的[反代方案](#https)，并把
> `VK_PUBLIC_BASE_URL` 配成那个 https 地址。
>
> **账号密码登录是团队版的主路径**，纯局域网部署用它就够了，不配任何 OAuth
> 提供方登录页也不会出现多余按钮。

`VK_ALLOW_OAUTH_SIGNUP` 默认关闭：第三方登录只能登录已存在的、已绑定的账号，
不会按 email 自动建号或自动合并账号（自动合并 = 谁先注册到那个邮箱谁就能劫持）。
绑定要在登录之后手工做。

---

## 备份与恢复

### 为什么不能直接 cp

数据库跑在 **WAL 模式**下。最近的写入躺在 `db.v2.sqlite-wal` 里，还没 checkpoint
回主库。单独 `cp db.v2.sqlite` 得到的是一个「少了最近一段时间全部写入」的旧快照，
而且它**能正常打开、`PRAGMA integrity_check` 也返回 `ok`**——这种损坏在真正需要
恢复的那天之前完全无法察觉。

`deploy/backup_test.sh` 里有一条测试把这件事钉死了：同一个库，直接 cp 主库只读到
3 行，`sqlite3 .backup` 读到全部 23 行。

`deploy/backup.sh` 一律用 SQLite 的在线备份 API（`.backup`），它自己处理 WAL 与
并发写，**服务在跑的时候也能安全备份，不需要停服务**。

### 备份

```bash
./deploy/backup.sh                          # 输出到 <数据目录>/backups
./deploy/backup.sh --out /mnt/nas/vk-backup --keep 30
```

- 输出 `vibe-kanban-<YYYYmmdd-HHMMSS>.tar.gz`
- 包内含 `db.v2.sqlite`（一致性快照）、`server.json`、`config.json`、
  `profiles.json`、`MANIFEST.txt`
- 备份产物会当场做一次 `integrity_check` 与 `tar -tzf` 回读，不通过就丢弃并报错，
  不会留下一个「看起来存在其实用不了」的包
- `--keep N`（或 `VK_BACKUP_KEEP`，默认 7）保留最近 N 份，多的删掉

**包里刻意不含本机凭据**：`credentials.json`、`machine_token`、
`server_ed25519_signing_key`、`trusted_ed25519_public_keys.json`、
`relay_host_credentials.json`。它们是这台机器的身份，进了备份包就等于把泄露面
从「一台机器」扩大到「每一份备份、每一个存备份的网盘」。代价是恢复到新机器后
需要重新登录云端账号、重新配对 MCP。

定时备份（macOS 用 launchd，Linux 用 systemd timer 或 cron）：

```cron
# 每天凌晨 3 点
0 3 * * * /path/to/deploy/backup.sh --out /mnt/nas/vk-backup --keep 30 >> /tmp/vk-backup.log 2>&1
```

### 恢复

**顺序是「停服务 → 恢复 → 起服务」，不能省。**

```bash
# 1. 停
launchctl bootout gui/$(id -u)/ai.bloop.vibe-kanban    # macOS
systemctl --user stop vibe-kanban                       # Linux

# 2. 恢复
./deploy/restore.sh /mnt/nas/vk-backup/vibe-kanban-20260917-030000.tar.gz

# 3. 起
launchctl kickstart -k gui/$(id -u)/ai.bloop.vibe-kanban
systemctl --user start vibe-kanban
```

**为什么必须先停**：服务进程持有 `db.v2.sqlite` 的连接，以及 `-wal` / `-shm` 两个
伴生文件。在它跑着的时候把主库换掉，进程手里的旧 WAL 会被当成新库的 WAL 继续用，
两边页号对不上，结果是**新库当场被写坏**。停掉再恢复，进程下次从干净状态重新打开
库，WAL 从零开始。

`restore.sh` 检测到服务在跑会直接拒绝执行并打印上面这两条命令。`--force` 能跳过
这个检查，但除非你确认检测误报，否则不要用。

恢复过程本身是保守的：

- 先校验备份包能解开、包里有 `db.v2.sqlite`、且它的 `integrity_check` 通过；
  任何一条不过就中止，**现有数据一点不动**
- 现有的 `db.v2.sqlite`（含 `-wal` / `-shm`）改名成 `.bak.<时间戳>` 留在原地，
  不删除。确认新库正常之后自己删
- 拷回之后再校验一次；这一步失败会自动把留底的旧库换回来
- `--db-only` 只恢复数据库，不覆盖 `server.json` / `config.json` / `profiles.json`

### 从个人版升级到团队版

升级是纯加法：新增几张表和一条固定用户行，历史需求的创建者字段不会被改写。

1. 先备份（`./deploy/backup.sh`）
2. 把 `VK_MODE` 改成 `team`，重启
3. 从日志里拿一次性初始化链接，创建管理员

切回 `personal` 同样不丢数据——**模式只影响鉴权，不影响数据**。
`VK_SQLITE_WAL=0` 可以回退 journal 模式。

### 自测

```bash
bash deploy/backup_test.sh
```

全程用临时目录，不碰真实数据目录。覆盖：基本往返、WAL 未 checkpoint 场景
（含「直接 cp 会丢数据」的对照组）、恢复往返、覆盖恢复留底、保留份数、
以及数据目录不存在 / 缺库 / 缺 `sqlite3` / 包损坏 / 包里的库损坏 / 不是本项目的
备份包这六种错误路径。

---

## 排查

**服务起不来，日志是空的**
二进制路径不对或没有执行权限。安装脚本会提前拦，手工写 plist / unit 时容易踩。
macOS 上还可能是 Gatekeeper 隔离：`xattr -d com.apple.quarantine ./vibe-kanban`。

**登录成功但立刻弹回登录页**
`VK_PUBLIC_BASE_URL` 配成了 `https://...` 但实际是明文 HTTP 访问。服务发的
Cookie 带 `Secure`，浏览器在 HTTP 页面上直接丢弃。去掉这个变量，或真的上 HTTPS。

**写操作返回 403**
来源校验没过。确认成员访问用的地址与后端看到的 `Host` 一致；反代改写了 `Host`
就把成员实际访问的来源加进 `VK_ALLOWED_ORIGINS`。

**一个人输错密码，所有人都被锁**
上了反代但没设 `VK_TRUST_PROXY=1`，限速按代理 IP 分桶。

**MCP 连不上但服务活着**
Linux 上 unit 里开了 `PrivateTmp=true`。改回 `false`。

**注销之后服务就停了（Linux）**
没开 linger：`sudo loginctl enable-linger "$(id -un)"`。

**团队模式下云端远程访问不通**
这是设计如此：团队模式不支持云端 relay，relay 请求一律 401。需要外网访问请用
[HTTPS 反代](#https)。
