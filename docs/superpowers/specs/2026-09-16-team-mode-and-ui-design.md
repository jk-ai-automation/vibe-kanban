# 团队版、本地认证与界面优化设计（P3）

> 状态：设计中｜日期：2026-09-16
> 上级方案：`docs/superpowers/specs/2026-09-15-vibe-kanban-optimization-roadmap.md`
> 前置：`docs/superpowers/specs/2026-09-16-personal-issue-flow-design.md`（个人版已落地，PR #1）

## 1. 目标

一套二进制、一份 SQLite，**不依赖 Docker、Postgres、ElectricSQL、云端**，同时支持：

- **个人版**：本机单人，免登录，行为与现在一致。
- **团队版**：一台机器跑服务端，约 5 人用浏览器连进来，各自有账号，能看到彼此的项目与需求。

并把界面改成贴合「需求 → 开发 → 评审 → 测试 → 完成」的研发测试流程。

## 2. 现状要点（调研结论，均已核对代码）

| 事实 | 位置 | 影响 |
|---|---|---|
| 本地 `/api/*` **无任何身份认证**，只有 Origin 同源校验 | `crates/server/src/routes/mod.rs:38-90`、`middleware/origin.rs:56-79` | 一旦监听 `0.0.0.0`，同网段任何浏览器都是全权限。团队版必须先补认证 |
| 本地 SQLite **没有** users/sessions/权限表 | `crates/db/migrations/` 80 个迁移里没有 | 需要新建，字段可复用已预留的 `creator_user_id`/`author_id` |
| 所有写入硬编码默认用户/组织；生产代码只有 4 处 | `local_project.rs:88`、`issue.rs:429`、`issue_side.rs:332`、`projections.rs:115`（其余 30+ 处在 `#[cfg(test)]`） | 换成请求上下文里的当前用户即可，**不需要改表结构** |
| 后端有 SSE `/api/events`，但**前端不消费它**；实时通道是 WebSocket `/api/issues/streams/ws` | `routes/events.rs:15-27`、`routes/issues.rs:21-77`、前端 `localCollections.ts:288-321` | Cookie 会自动带上，但 WebSocket 不受同源策略保护，必须在握手时校验 Origin（见 6.3） |
| 变更钩子表白名单：workspaces、execution_processes、scratch、issues、project_statuses、issue_comments | `events.rs:87-141` | 新表若要推送需登记；且 `id` 必须是第 0 列 |
| 前端本地请求是裸 `fetch`，唯一收口在 `localApiTransport.ts:111-116` | 同源 fetch 默认就带 Cookie，**不需要** `credentials:'include'`（反而会触发 CORS 预检）；要加的是 CSRF 头 |
| 监听地址默认 `127.0.0.1`，端口来自 `BACKEND_PORT`/`PORT` | `crates/server/src/main.rs:115-121` | 团队版显式设 `0.0.0.0`，不是硬编码死锁 |
| 前端 dist 编译期嵌入二进制 | `routes/frontend.rs:7-9`（RustEmbed） | 部署就是「拷一个可执行文件」，天然满足「无 Docker 直接部署」 |
| 数据目录 `ProjectDirs::from("ai","bloop","vibe-kanban")` | `crates/utils/src/assets.rs:23-27` | 备份 = 备份该目录 |
| SQLite 用 `journal_mode=Delete` + `busy_timeout(10s)` + 应用层重试 | `crates/db/src/lib.rs:73-98`、`models/db_retry.rs` | 多人并发写要评估切 WAL |
| 云端账号体系（OAuth/JWT/组织）在 `crates/remote`，绑定 Postgres | `crates/remote/src/auth/*` | **不复用**，本地重新实现精简版 |
| `local-build.sh` 把 `VK_SHARED_API_BASE` 硬编码为官方云端 | `local-build.sh` | 自建部署要参数化 |

## 3. 范围

**本期做**

1. 本地用户体系：`local_users`、`local_sessions`、`local_user_identities`（第三方账号绑定）。
2. 密码认证（Argon2id）+ HttpOnly Cookie 会话；登录、登出、改密、首启建管理员。
3. 第三方登录：飞书、Lark、Google 三个 OAuth2 提供方（统一抽象 + 三份预置配置），未配置凭据时自动隐藏入口。
4. 运行模式：`personal`（默认，免登录）/ `team`（强制登录），一个二进制两种模式。
5. 成员管理：管理员建号/停用/重置密码/改角色；邀请链接注册。
6. 身份落地：写入记录时用真实用户；个人版继续用固定本机用户，历史数据不受影响。
7. 鉴权中间件覆盖 `/api/*`、SSE 与 WebSocket；写操作做 CSRF 双重校验；本机工具用 `X-VK-MACHINE-TOKEN` 豁免。
8. 界面优化：信息架构、流程化看板、需求详情三段式、工作区与需求联动、筛选与快捷键、团队版的人员元素。
9. 部署：一键启动脚本、launchd/systemd 模板、首启向导、备份与恢复脚本、局域网访问与 HTTPS 说明。

**本期不做**（明确写进文档，避免期待落空）

- 项目级可见性隔离（团队内全员可见，与用户既定需求一致）。
- 企业 SSO（LDAP/SAML）。
- 云端 relay/Electric 相关能力；`crates/remote` 保持原样不动。
- 端到端加密、审计日志留存策略、多租户计费。
- 测试结果回填（等 atp 的 `results.json` 对接，下一期）。

## 4. 架构

```
浏览器（成员 A/B/C…）
  │  Cookie: vk_session（HttpOnly, SameSite=Lax）
  ▼
单个可执行文件 vibe-kanban
  ├─ 静态前端（RustEmbed，编译期嵌入）
  ├─ /api/local-auth/*    登录、登出、当前用户、改密、OAuth 回调
│                       （`/api/auth/*` 已被云端 OAuth 路由占用，见 routes/oauth.rs:81-92）
  ├─ /api/admin/users/*   成员管理（仅 admin）
  ├─ 鉴权中间件           team 模式强制会话；personal 模式注入本机用户
  ├─ /api/local/*         需求、项目、状态列（写入带真实用户）
  ├─ /api/workspaces/*    工作区与编码智能体
  ├─ /api/events          SSE 变更推送（同样鉴权）
  └─ /api/issues/streams/ws  WebSocket 实时通道（握手校验 Origin，防跨站劫持）
        ▼
   SQLite（单文件，WAL）
```

一台机器、一个进程、一个数据库文件。团队成员访问 `http://<主机名或IP>:<端口>`。

## 5. 数据模型

新增迁移 `20260917000000_add_local_auth.sql`（`id` 必须是第 0 列，沿用钩子约定）：

| 表 | 字段 |
|---|---|
| `local_users` | id、username（唯一，小写规范化）、display_name、email、password_hash（可空，仅第三方登录时为空）、role（`admin`/`member`）、status（`active`/`disabled`）、avatar_color、created_at、updated_at、last_login_at |
| `local_sessions` | id、user_id、token_hash（存哈希，不存明文）、created_at、expires_at、last_seen_at、user_agent、ip、revoked_at |
| `local_user_identities` | id、user_id、provider（`feishu`/`lark`/`google`）、subject（提供方用户 ID）、email、created_at；`(provider, subject)` 唯一 |
| `local_invites` | id、code_hash、role、created_by、expires_at、used_by、used_at |

说明：
- 密码哈希用 Argon2id（参数：m=19456、t=2、p=1，OWASP 推荐档），不自造。
- 会话令牌 32 字节随机，返回给浏览器的是明文，库里只存 SHA-256；有效期 30 天，滑动续期，单用户可有多设备会话。
- 个人版固定用户 `DEFAULT_USER_ID` 作为一条 `local_users` 记录写入（username=`local`，role=`admin`），保证外键与历史数据一致。
- 迁移要把已有记录里的 `creator_user_id`/`author_id` 保持原值（就是 `DEFAULT_USER_ID`），不做改写。

## 6. 认证与鉴权

### 6.1 模式

| 模式 | 触发 | 行为 |
|---|---|---|
| `personal` | 默认；或 `VK_MODE=personal` | 不要求登录；中间件注入本机用户；隐藏登录/成员管理入口；监听 `127.0.0.1` |
| `team` | `VK_MODE=team` | `/api/*` 全部要求有效会话（`/api/local-auth/login`、`/api/local-auth/providers`、健康检查除外）；首次启动强制走「创建管理员」向导；建议监听 `0.0.0.0`。**relay 请求一律 401——团队模式不支持云端 relay** |

模式来自环境变量与配置文件 `asset_dir()/server.json`，配置文件优先级低于环境变量。前端启动时从 `/api/local-auth/bootstrap` 读取当前模式与可用登录方式，不再依赖构建期的 `VITE_VK_DATA_SOURCE`。

### 6.2 会话

- 登录成功 → `Set-Cookie: vk_session=<明文令牌>; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000`，HTTPS 下追加 `Secure`。**用 `Lax` 而非 `Strict`**：`Strict` 会让 OAuth 提供方跳回来的那次导航带不上 Cookie。`Lax` 的安全前提是「GET 不做写操作」，落地前必须扫一遍现有路由确认。
- 中间件：取 Cookie → SHA-256 → 查 `local_sessions` → 校验未撤销、未过期 → 取用户 → 校验 `status=active` → 挂进 request extension。
- 每次命中更新 `last_seen_at`（节流：距上次超过 5 分钟才写库，避免每请求一次写）。
- 登出撤销当前会话；改密撤销该用户全部会话。
- 失败一律返回 401 且**不区分**「用户不存在」与「密码错误」；登录失败按用户名 + IP 限速（10 次/5 分钟，指数退避），避免暴力破解。

### 6.3 CSRF

Cookie 会话天生受 CSRF 威胁。三道防线：
1. `SameSite=Lax`（不能用 `Strict`，见 6.2）。
2. 现有 Origin 校验对写方法（POST/PATCH/PUT/DELETE）强制要求 `Origin` 存在且匹配（当前实现允许缺失 Origin 时放行，需收紧）。
3. 登录时下发 `vk_csrf` 非 HttpOnly Cookie，前端写操作带 `X-VK-CSRF` 头，服务端比对（双提交）。
4. **WebSocket 单独防跨站劫持（CSWSH）**：浏览器不对 WebSocket 施加同源策略，带 Cookie 的升级请求会被自动发出。`/api/issues/streams/ws` 等端点握手时强制校验 `Origin`，不匹配直接拒绝升级。
5. **本机工具豁免**：`vibe-kanban-mcp` 用裸 `reqwest` 直连本地端口，不带 Origin 也不带 Cookie（`crates/mcp/src/task_server/mod.rs:59-77`）。收紧 Origin 后它会被打死，因此引入本机令牌 `X-VK-MACHINE-TOKEN`（随端口文件落在数据目录，仅本机可读）作为等价凭据。

### 6.4 第三方登录

> 以下端点与字段已查官方文档核实（2026-09-16），核实结论见本节末尾的「已核实 / 未核实」。**实现时不得凭记忆写死，以官方文档与实测为准。**

统一抽象 `OAuthProvider { authorize_url, token_url, token_body_format, userinfo_url, scopes, subject_field, email_field, name_field }`，预置三份：

| 提供方 | 授权端点 | 换令牌端点 | 用户信息端点 |
|---|---|---|---|
| `feishu` | `https://accounts.feishu.cn/open-apis/authen/v1/authorize` | `https://accounts.feishu.cn/oauth/v3/token`（POST，form-urlencoded） | `https://open.feishu.cn/open-apis/authen/v1/user_info` |
| `lark` | `https://accounts.larksuite.com/open-apis/authen/v1/authorize` | `https://open.larksuite.com/open-apis/authen/v2/oauth/token`（POST，JSON） | `https://open.larksuite.com/open-apis/authen/v1/user_info` |
| `google` | `https://accounts.google.com/o/oauth2/v2/auth` | `https://oauth2.googleapis.com/token` | `https://openidconnect.googleapis.com/v1/userinfo` |

要点：

- **参数名是 `client_id` 不是 `app_id`**（飞书后台展示为 "App ID"，但 URL 参数是 `client_id`）。
- **飞书的 v2 换令牌端点已被官方标注弃用**，用 v3；Lark 侧目前官方文档仍是 v2 且无弃用提示，两边版本不同步，因此 token 端点必须做成**可配置项**。
- **飞书/Lark 的认证类接口是扁平结构**（`code` 与 `access_token` 同层），**不套** `{code,msg,data}` 信封——不要写一个通用的「剥 data 层」逻辑套所有飞书接口。
- **稳定身份标识**：
  - 飞书/Lark 用 **`union_id`**。`open_id` 是应用级的、换应用凭据就变；`user_id` 在管理员删号后可能被新用户复用；email/mobile 官方明文说「未经用户本人实时验证，不建议作为业务系统的登录凭证」。
  - Google 用 **`sub`**。官方原文：`Don't use the email field as a unique identifier for a user. Always use the sub field.`
  - 库里同时保存 `open_id` 备查，但绑定键只用 `union_id`/`sub`。
- PKCE：飞书支持但不强制（应用一律视为机密客户端，必须带 `client_secret`）；Google 对服务端机密客户端不强制。实现时带上 PKCE 无害，优先做。
- 凭据从 `asset_dir()/server.json` 或环境变量读取；未配置的提供方不出现在登录页。
- 回调 `/api/local-auth/oauth/:provider/callback`：校验 `state`（服务端生成、一次性、10 分钟过期）→ 换 token → 拉用户信息 → 按 `(provider, subject)` 查 `local_user_identities`：
  - 已绑定 → 建会话登录。
  - 未绑定但 email 命中已有用户 → 需该用户已登录状态下手动绑定，**不自动合并**（防账号劫持）。
  - 未绑定且允许自助注册（配置项 `allow_oauth_signup`，默认关）→ 建新用户，角色 `member`。
  - 否则 → 提示「请联系管理员开通」。
- 全流程不记录 access token，只记 `subject`。授权码有效期短且一次性，令牌有效期以接口实际返回的 `expires_in` 为准，**不要硬编码任何时长数字**。
- 限流错误码（飞书 `99991400`/`1000004`/`1000005`）官方建议指数退避。
- 无法在本机验证真实提供方，因此测试用本地 mock OIDC 服务覆盖成功、state 失效、token 失败、userinfo 缺字段、subject 冲突、邮箱撞号不自动合并等分支。

#### 回调地址限制对部署形态的影响（重要）

Google 官方规定：`Redirect URIs must use the HTTPS scheme, not plain HTTP` 且 **`Hosts cannot be raw IP addresses`**，只有 `localhost`/`127.0.0.1`/`[::1]` 是例外。

因此在「局域网 `http://192.168.x.x:8080`」这种部署形态下：

| 登录方式 | 局域网 HTTP 部署 | 说明 |
|---|---|---|
| **账号密码** | ✅ 可用 | **这是团队版的主路径**，不受任何回调限制 |
| 飞书 / Lark | ⚠️ 待实测 | 官方文档没有明文规定是否强制 HTTPS 或禁止裸 IP；社区实践显示 `http://localhost` 可注册。**上线前必须在开发者后台实测能否填入局域网地址** |
| Google | ❌ 不可用 | 裸 IP 被明确禁止。要用 Google 登录必须给服务套一个真实域名 + HTTPS（见 8.5 的反向代理方案） |

结论：**账号密码登录必须做到完全自洽可用**，第三方登录是加分项；登录页按配置与可用性动态显示，不可用时不出现入口。

#### 已核实 / 未核实

已核实（有官方原文）：三家的授权/令牌/用户信息端点与参数名、飞书 v2 弃用、飞书认证接口的扁平结构、`union_id`/`open_id`/`user_id` 的稳定性差异与官方推荐、Google 必须用 `sub`、Google 的裸 IP 与 HTTPS 限制、飞书常见错误码、Google 对机密客户端不强制 PKCE。

**未核实（实现前必须确认，不得当作事实）**：飞书/Lark 的 redirect_uri 是否强制 HTTPS、是否允许局域网裸 IP（最大缺口）；Lark 是否也有 v3 令牌端点；Lark 的 user_info 响应字段是否与飞书一致；Lark 是否支持 PKCE；飞书/Lark 拿到 `union_id`/`name`/`email` 所需的最小权限点字符串（需在应用后台权限管理页核对，不要照抄）；Google 「Web application」与「Desktop app」客户端类型对 localhost 回调的支持范围（两份官方文档表述有出入）。

### 6.5 权限

| 角色 | 能力 |
|---|---|
| `admin` | 全部；另加成员管理、邀请码、改他人角色与状态 |
| `member` | 项目、需求、工作区的读写；不能管理成员 |

团队内所有项目对所有成员可见（既定需求）。删除项目、删除他人评论仅 admin 或本人。

## 7. 界面优化

原则：**围绕一条需求从提出到验收的流程组织界面**，而不是围绕数据表。

### 7.1 信息架构

```
顶栏：项目切换器 ｜ 全局搜索(/) ｜ 新建(n) ｜ 主题 ｜ 用户菜单(团队版显示头像与成员入口)
左侧导航：看板 ｜ 我的需求 ｜ 工作区 ｜ 测试(占位，下一期接 atp) ｜ 设置
主区：随导航切换
右侧栏：上下文面板（需求详情/工作区详情），可折叠
```

### 7.2 看板

- 泳道即流程阶段：待规划 → 待开发 → 开发中 → 待评审 → 测试中 → 已完成（`project_statuses.stage_type` 已有，新增 `test` 阶段）。
- 卡片信息密度重排：`simple_id` + 标题为主，底部一行状态徽标——关联工作区运行状态（跑中/已停/失败）、PR 状态（草稿/待评审/已合并）、测试结果占位、标签、负责人头像（团队版）。
- 列头显示计数与 WIP 提示（超过 5 张变色，只提示不阻止）。
- 拖拽落列时若该阶段有前置条件（例如「待评审」要求有工作区），给一次确认而不是静默失败。
- 空列有明确的下一步引导按钮（「新建需求」「从需求创建工作区」）。

### 7.3 需求详情

三段式，标签页切换：
1. **概况**：描述、标签、负责人、优先级、时间、评论与活动流。
2. **开发**：关联工作区列表（分支、智能体、运行状态、diff 统计）、一键创建工作区、跳转编辑器、PR 链接与状态。
3. **测试**：本期为占位面板，显示「尚未接入」，结构按 atp 的 `results.json` 预留（通过率、失败清单、报告链接）。

### 7.4 工作区

- 列表按需求分组，显示所属需求 `simple_id`、分支、智能体状态、最近活动时间。
- 详情页顶部固定一条「需求 → 工作区 → PR」的路径条，随时能跳回需求。

### 7.5 交互细节

- 快捷键：`/` 搜索、`n` 新建需求、`j/k` 上下移动、`Enter` 打开、`e` 编辑、`Esc` 关闭面板；提供快捷键帮助弹窗（`?`）。
- 筛选条件（状态、标签、负责人、关键字）写进 URL query，可分享可刷新。
- 列表密度切换（舒适/紧凑）记在本地存储。
- 加载态一律用骨架屏而不是整页 spinner；空态、错误态都要有具体文案与下一步动作。
- 沿用既有设计令牌（`text-high/normal/low`、`bg-primary/secondary/panel`、`brand` 橙）与 View/Container/UI 三层约定，暗色模式必须同时适配。
- 团队版才出现的元素（头像、指派、成员管理）在个人版隐藏，不留空占位。

## 8. 部署

### 8.1 形态

一个可执行文件 + 一个数据目录，无容器、无外部数据库。

```
vibe-kanban              # 含前端静态资源
~/Library/Application Support/ai.bloop.vibe-kanban/   # macOS
~/.local/share/vibe-kanban/                            # Linux
  ├─ db.v2.sqlite        # 全部数据
  ├─ server.json         # 模式、监听、OAuth 凭据
  └─ …                   # 既有的 config/profiles/credentials
```

### 8.2 启动

```bash
# 个人版（默认）
vibe-kanban

# 团队版：监听全网卡
VK_MODE=team HOST=0.0.0.0 BACKEND_PORT=8080 vibe-kanban
```

首次以 `team` 模式启动且库里没有管理员时，控制台打印一次性初始化链接（含一次性令牌），浏览器打开后创建管理员账号；令牌 30 分钟过期，用后失效。

### 8.3 服务化

提供模板与安装脚本：
- macOS：`deploy/launchd/ai.bloop.vibe-kanban.plist` + `deploy/install-macos.sh`
- Linux：`deploy/systemd/vibe-kanban.service` + `deploy/install-linux.sh`
- 两者都设置 `Restart=always`、日志落到数据目录下 `logs/`。

### 8.4 备份与恢复

`deploy/backup.sh`：用 `sqlite3 .backup` 做一致性备份（不是直接 cp），打包数据目录，保留最近 N 份；`deploy/restore.sh` 反向。文档写明「停服务 → 恢复 → 起服务」。

### 8.5 局域网与 HTTPS

- `VK_ALLOWED_ORIGINS` **通常不需要配**：Origin 校验是「Origin 与 Host 对得上就放行」（`middleware/origin.rs:76-100`），成员直接用 `http://<IP>:<端口>` 访问时两者天然一致。真正需要配它的是**反代改写了 Host** 或**走别名域名**的场景。
- 明文 HTTP 下 Cookie 无 `Secure`，仅建议在受信任的内网使用；需要 HTTPS 时给出 Caddy 反代的十行配置示例（自动证书），并说明此时要设 `VK_TRUST_PROXY=1` 以便取真实客户端 IP 做限速。

### 8.6 构建

`local-build.sh` 的 `VK_SHARED_API_BASE` 参数化（默认空＝纯本地，不连任何云端），新增 `pnpm run build:selfhost` 产出自建部署包（可执行文件 + deploy/ 模板 + README）。

### 8.7 与云端 relay 的关系

现有 relay 中间件（主机对主机签名）位于会话中间件之内，会话层看不到它的验证结果。团队模式下 relay 请求一律 401，即**团队模式不支持把本机暴露给云端 relay**；需要外网访问用 8.5 的反向代理方案。

## 9. 并发与数据安全

- SQLite 切 **WAL**（`journal_mode=WAL`、`synchronous=NORMAL`），并发读不再被写阻塞；需验证：变更钩子在 WAL 下行为一致、Windows 文件锁、备份脚本用 `.backup` 而非 cp。切换要可回退（配置项）。
- 连接池显式设上限（读写分离不做，先设 `max_connections=16`）。
- 保留现有 `db_retry` 重试。
- 会话表定期清理过期记录（启动时 + 每 6 小时）。

## 10. 风险与对策

| 风险 | 对策 |
|---|---|
| 监听 `0.0.0.0` 后被内网未授权访问 | 团队版强制登录；未创建管理员前只开放初始化路由；文档强调不要暴露到公网 |
| Cookie 会话的 CSRF | SameSite=Lax（Strict 会打断 OAuth 回调，见 6.2）+ 写方法与 WebSocket 升级的 Origin 强校验 + 双提交令牌，三者都要有测试 |
| OAuth 无法在本机验证真实提供方 | 用 mock OIDC 服务覆盖分支；文档写明上线前需用真实凭据实测一次 |
| 自动按 email 合并账号导致劫持 | 明确不自动合并，只能登录后手动绑定 |
| WAL 切换引发钩子或备份问题 | 单独一批任务验证 + 可回退配置项 |
| 个人版行为回退 | 个人版路径全部保留并有回归测试；模式切换不改数据 |
| 界面大改导致既有功能丢失 | 逐屏对照改造，不推倒重来；每屏有交互测试 |
| 密码学实现出错 | 只用成熟库（argon2、rand、subtle 常量时间比较），不自造 |

## 11. 验收标准

1. 个人版：`vibe-kanban` 直接启动，免登录，现有链路（建项目 → 建需求 → 拖拽 → 创建工作区 → 合并 → 需求完成）全部照常。
2. 团队版：`VK_MODE=team HOST=0.0.0.0` 启动 → 初始化管理员 → 管理员建 2 个成员 → 三个浏览器分别登录 → 各自建需求、互相可见 → 一侧改动另一侧 2 秒内更新（SSE）。
3. 未登录访问任意 `/api/*` 返回 401；伪造 Cookie、过期会话、被停用用户、跨站请求全部被拒，且各有测试。
4. 三个 OAuth 提供方在 mock IdP 下走通登录与绑定；未配置凭据时登录页不出现对应按钮。
5. 部署包在一台干净机器上：解压 → 跑安装脚本 → 服务自启 → 浏览器可访问；备份与恢复脚本可用。
6. `cargo test --workspace`、`pnpm run check`、`pnpm run lint`、前端 Vitest 全绿；新增后端逻辑有单测，新增前端交互有测试。
7. 暗色与浅色模式下所有新界面无对比度问题；窄屏（1280px）不出现横向滚动。
