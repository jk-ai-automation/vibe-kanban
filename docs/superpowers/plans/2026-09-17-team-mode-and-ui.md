# 团队版、本地认证与界面优化 实施计划（P3）

> **给智能体执行者：** 用 `superpowers:subagent-driven-development` 或 `superpowers:executing-plans` 按任务顺序实施。步骤用复选框（`- [ ]`）跟踪。
>
> **假设你对本仓库零了解。** 每一步都写了精确的文件路径、行号依据、命令与预期输出。凡是本计划标注「**待核实**」的地方，先核实再动手，不要按计划里的猜测写代码。

**目标：** 一套二进制 + 一份 SQLite，无 Docker / Postgres / ElectricSQL / 云端，同时支持个人版（免登录，行为不变）与团队版（约 5 人，各自账号，浏览器访问），并把界面改造成贴合「需求 → 开发 → 评审 → 测试 → 完成」的流程。

**设计文档：** `docs/superpowers/specs/2026-09-16-team-mode-and-ui-design.md`（263 行，执行前通读）。

**技术栈：** Rust 2024 edition（axum 0.8.4 / sqlx 0.8.6 SQLite / tokio / ts-rs）、TypeScript + React 18、TanStack Router + TanStack DB、Vitest 3、pnpm 10。

---

## 0. 阅读顺序（执行前必读）

1. 本计划第 1～4 节（约定、已核实事实、对设计文档的纠正、文件清单）。
2. 设计规格：`docs/superpowers/specs/2026-09-16-team-mode-and-ui-design.md`
3. 前一期计划（同一套代码风格与测试风格）：`docs/superpowers/plans/2026-09-16-personal-issue-flow.md`
4. 仓库约定：`CLAUDE.md`、`packages/local-web/AGENTS.md`、`docs/AGENTS.md`

**不改动 `crates/remote`。** 它被根 `Cargo.toml:34` 的 `exclude = ["crates/remote", "crates/relay-tunnel"]` 排除在默认 workspace 之外，只在 `pnpm run backend:check` / `backend:lint` 里单独编译；本机无 Docker、无 Postgres，`crates/remote` 的 CI job 在本 fork 因 `vars.HAS_PRIVATE_DEPLOY_KEY` 未配置而跳过（`.github/workflows/test.yml:227`）。因此**不得修改 `crates/api-types` 中已有类型的字段**，也不要手改 `shared/remote-types.ts`（它由 remote crate 生成）。

---

## 1. 跨任务共享约定（务必逐字一致）

### 1.1 路由前缀

| 用途 | 路径 | 说明 |
|---|---|---|
| 本地账号体系 | `/api/local-auth/...` | **不能用 `/api/auth/...`**，见 §3.1 |
| 成员管理 | `/api/admin/...` | 仓库内当前无任何 `/api/admin` 路由（已 grep 确认） |
| 既有本地数据接口 | `/api/local/...` | `crates/server/src/routes/local_projects/mod.rs:63` |
| 需求实时流（WebSocket） | `/api/issues/streams/ws?project_id=<uuid>` | `crates/server/src/routes/issues.rs:76` |
| SSE（当前前端不消费） | `/api/events` | `crates/server/src/routes/events.rs:27` |

完整端点表：

| 方法 | 路径 | 是否需要会话 | 任务 |
|---|---|---|---|
| GET | `/api/local-auth/bootstrap` | 否 | A |
| POST | `/api/local-auth/login` | 否 | A |
| POST | `/api/local-auth/logout` | 是 | A |
| GET | `/api/local-auth/me` | 是 | A |
| POST | `/api/local-auth/password` | 是 | A |
| GET | `/api/local-auth/setup` | 否（需一次性令牌） | D |
| POST | `/api/local-auth/setup` | 否（需一次性令牌） | D |
| POST | `/api/local-auth/invites/accept` | 否 | D |
| GET | `/api/admin/users` | 是（admin） | D |
| POST | `/api/admin/users` | 是（admin） | D |
| PATCH | `/api/admin/users/{id}` | 是（admin） | D |
| POST | `/api/admin/users/{id}/password` | 是（admin） | D |
| GET | `/api/admin/invites` | 是（admin） | D |
| POST | `/api/admin/invites` | 是（admin） | D |
| DELETE | `/api/admin/invites/{id}` | 是（admin） | D |
| GET | `/api/local-auth/oauth/{provider}/start` | 否 | E |
| GET | `/api/local-auth/oauth/{provider}/callback` | 否 | E |
| POST | `/api/local-auth/oauth/{provider}/bind` | 是 | E |
| DELETE | `/api/local-auth/identities/{id}` | 是 | E |
| GET | `/api/health` | 否 | A（从受保护集合移出） |

### 1.2 Cookie 与请求头

| 名称 | 属性 | 用途 |
|---|---|---|
| `vk_session` | `HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000`（HTTPS 时加 `Secure`） | 会话明文令牌（库里只存 SHA-256） |
| `vk_csrf` | `SameSite=Lax; Path=/; Max-Age=2592000`（**非** HttpOnly，HTTPS 时加 `Secure`） | 双提交令牌 |
| `X-VK-CSRF` | 请求头 | 前端从 `vk_csrf` 读出后回填 |
| `X-VK-MACHINE-TOKEN` | 请求头 | 本机进程（MCP）在 team 模式下的免会话凭据，见 §3.4 |

**SameSite 用 `Lax` 而不是设计文档写的 `Strict`**，理由见 §3.2。

### 1.3 环境变量与配置项

配置文件 `asset_dir()/server.json`；环境变量优先级**高于**配置文件。

| 环境变量 | server.json 字段 | 默认值 |
|---|---|---|
| `VK_MODE` | `mode` | `personal`（合法值 `personal` / `team`，其它值 → 启动失败） |
| `VK_ALLOW_OAUTH_SIGNUP` | `allow_oauth_signup` | `false` |
| `VK_SESSION_TTL_DAYS` | `session_ttl_days` | `30` |
| `VK_SQLITE_WAL` | `sqlite_wal` | `true`（任务 F 落地；设 `0` 回退到 Delete） |
| `VK_TRUST_PROXY` | `trust_proxy` | `false` |
| `VK_PUBLIC_BASE_URL` | `public_base_url` | 无（OAuth 回调拼接用） |
| `VK_OAUTH_FEISHU_CLIENT_ID` / `_SECRET` | `oauth.feishu.{client_id,client_secret}` | 无 |
| `VK_OAUTH_LARK_CLIENT_ID` / `_SECRET` | `oauth.lark.{client_id,client_secret}` | 无 |
| `VK_OAUTH_GOOGLE_CLIENT_ID` / `_SECRET` | `oauth.google.{client_id,client_secret}` | 无 |
| 既有 | — | `VK_ALLOWED_ORIGINS`（`crates/server/src/middleware/origin.rs:142`）、`HOST`（`main.rs:115`）、`BACKEND_PORT`/`PORT`（`main.rs:96-97`） |

### 1.4 Rust 类型与函数名

| 名称 | 定义位置 |
|---|---|
| `ServerMode { Personal, Team }` | `crates/services/src/services/server_settings.rs` |
| `ServerSettings` | 同上 |
| `LocalAuthRuntime` | `crates/services/src/services/local_auth/runtime.rs` |
| `hash_password` / `verify_password` | `crates/services/src/services/local_auth/password.rs` |
| `generate_session_token` / `hash_session_token` / `build_session_cookie` / `parse_cookie` | `crates/services/src/services/local_auth/token.rs` |
| `LoginRateLimiter` | `crates/services/src/services/local_auth/rate_limit.rs` |
| `OAuthProviderConfig` / `OAuthProviderId` | `crates/services/src/services/local_auth/oauth.rs` |
| `LocalUser` / `LocalUserRole` / `LocalUserStatus` / `LocalUsers` | `crates/db/src/models/local_user.rs` |
| `LocalSession` / `LocalSessions` / `LocalUserIdentity` / `LocalUserIdentities` / `LocalInvite` / `LocalInvites` | `crates/db/src/models/local_auth.rs` |
| `CurrentUser` / `require_local_session` | `crates/server/src/middleware/local_session.rs` |
| `DEFAULT_USER_ID` / `DEFAULT_ORGANIZATION_ID` | 已存在：`crates/db/src/models/local_project.rs:9-11` |

**命名冲突警告：** `services::services::auth::AuthContext` 已被云端 OAuth 占用（`crates/services/src/services/auth.rs:9`），新代码**不得**再叫 `AuthContext`。

### 1.5 数据库列名（迁移 `20260917000000_add_local_auth.sql`）

`id` 必须是第 0 列（沿用 `crates/services/src/services/events.rs:89` 的 `get_old_column_value(0)` 约定，即使这四张表暂不参与钩子）。

```
local_users(id, username, display_name, email, password_hash, role, status,
            avatar_color, created_at, updated_at, last_login_at)
local_sessions(id, user_id, token_hash, created_at, expires_at, last_seen_at,
               user_agent, ip, revoked_at)
local_user_identities(id, user_id, provider, subject, email, created_at)
local_invites(id, code_hash, role, created_by, expires_at, used_by, used_at, created_at)
```

### 1.6 前端类型与模块

| 名称 | 位置 |
|---|---|
| `BootstrapResponse` / `LoginRequest` / `LoginResponse` / `CurrentUserResponse2` 等 DTO | `crates/server/src/routes/local_auth/mod.rs`，经 `crates/server/src/bin/generate_types.rs` 生成到 `shared/types.ts` |
| `configureRuntimeMode` / `getRuntimeMode` | `packages/web-core/src/shared/lib/local/runtimeMode.ts`（新建） |
| `readCsrfToken` | `packages/web-core/src/shared/lib/local/csrf.ts`（新建） |
| `LocalSessionProvider` | `packages/web-core/src/shared/providers/auth/LocalSessionProvider.tsx`（新建） |
| 既有数据源开关 | `packages/web-core/src/shared/lib/local/dataSource.ts`（16 行，全文见 §2.7） |

---

## 2. 已核实的现状（全部带 文件:行号，执行时可直接跳过去看）

### 2.1 本地 `/api/*` 没有任何用户认证

`crates/server/src/routes/mod.rs:38-90` 是唯一的路由装配点：

```rust
38  pub fn router(deployment: DeploymentImpl) -> IntoMakeService<Router> {
39      let relay_signed_routes = Router::new()
40          .route("/health", get(health::health_check))
41          .merge(config::router())
...
63          .nest("/attachments", attachments::routes())
64          .layer(axum::middleware::from_fn_with_state(
65              deployment.clone(),
66              middleware::sign_relay_response,
67          ))
68          .layer(axum::middleware::from_fn_with_state(
69              deployment.clone(),
70              middleware::require_relay_request_signature,
71          ))
72          .with_state(deployment.clone());
73
74      let api_routes = Router::new()
75          .merge(relay_auth::router())
76          .merge(host_relay::router(&deployment))
77          .merge(relay_signed_routes)
78          .layer(ValidateRequestHeaderLayer::custom(
79              middleware::validate_origin,
80          ))
81          .layer(axum::middleware::from_fn(middleware::log_server_errors))
82          .with_state(deployment);
83
84      Router::new()
85          .route("/", get(frontend::serve_frontend_root))
86          .route("/{*path}", get(frontend::serve_frontend))
87          .nest("/api", api_routes)
88          .layer(CompressionLayer::new())
89          .into_make_service()
90  }
```

只有 Origin 同源校验 + relay 请求签名，**没有用户身份**。

### 2.2 Origin 中间件当前会放行缺失 Origin 的请求

`crates/server/src/middleware/origin.rs:48-50`：

```rust
48      let Some(origin) = get_origin_header(req) else {
49          return Ok(());
50      };
```

对应测试 `no_origin_header_allows_request`（`origin.rs:175-179`）。`is_relay_request`（`origin.rs:99-104`）让带 `X-Vibe-Relay: 1` 头的请求整体跳过 Origin 校验（`origin.rs:44-46`）。`VK_ALLOWED_ORIGINS` 解析在 `origin.rs:139-152`，用 `OnceLock` 只读一次。

### 2.3 数据库

- 连接参数：`crates/db/src/lib.rs:77-86`，`journal_mode(SqliteJournalMode::Delete)`（:84）+ `busy_timeout(BUSY_TIMEOUT)`（:85，`BUSY_TIMEOUT = 10s`，:73）。
- 连接池：`DBService::new()`（:94-98）用 `SqlitePool::connect_with`，**没有显式 max_connections**（sqlx 默认 10）。带钩子的那条路径是 `create_pool`（:136-165）。
- 生产装配点：`crates/local-deployment/src/lib.rs:139-147`
  ```rust
  139      let db = {
  140          let hook = EventService::create_hook(
  141              events_msg_store.clone(),
  142              events_entry_count.clone(),
  143              DBService::new().await?, // Temporary DB service for the hook
  144          );
  145          DBService::new_with_after_connect(hook).await?
  146      };
  ```
- 测试库工厂：`crates/db/src/test_support.rs`，`TestDb::new()` 建临时目录 + `DBService::new_at_path`，`test_db.pool()` 取 `&SqlitePool`。**所有新测试都用它。**
- 重试：`crates/db/src/models/db_retry.rs` 的 `retry_on_busy`（:49）、`is_unique_violation`（:26）、`is_retryable_db_error`（:13）。
- 现有迁移 78 个，最新两个是 `20260916000000_add_local_issue_tracking.sql`、`20260916010000_issue_number_sequence_and_indexes.sql`。

### 2.4 sqlx 离线数据

- `crates/db/.sqlx`（143 个 json）与 `crates/server/.sqlx`（2 个 json）。
- `scripts/prepare-db.js:14` 只 `chdir` 到 `crates/db`，**只重新生成 `crates/db/.sqlx`**；CI 的 `npm run prepare-db:check`（`.github/workflows/test.yml:219`）也只检查它。
- **结论（硬约束）：本期所有新的 `sqlx::query!` / `query_as!` 宏调用一律写在 `crates/db/src/models/` 下**，不要在 `crates/server` 里新增编译期校验的 SQL，否则需要手工重生成 `crates/server/.sqlx`，而现有脚本不覆盖它。

### 2.5 身份硬编码点（生产代码只有 4 处，其余全在测试里）

| 位置 | 内容 |
|---|---|
| `crates/db/src/models/local_project.rs:88` | `let organization_id = DEFAULT_ORGANIZATION_ID;`（建项目时忽略请求里的 organization_id） |
| `crates/db/src/models/issue.rs:429` | `Issues::create` 把 `super::local_project::DEFAULT_USER_ID` 绑给 `creator_user_id`（SQL 在 :388-412，`$16` 位） |
| `crates/db/src/models/issue_side.rs:332` | `IssueComments::create` 把 `DEFAULT_USER_ID` 绑给 `author_id`（SQL 在 :318-338，`$3` 位） |
| `crates/server/src/routes/local_projects/projections.rs:115` | `owner_user_id: DEFAULT_USER_ID`（workspaces 投影） |

其余 `DEFAULT_USER_ID` / `DEFAULT_ORGANIZATION_ID` 出现点（`issue_side.rs:465,480,696`、`issue.rs:928,944`、`local_project_status.rs:275,284`、`local_project.rs:241,247,331`、`workspace.rs:748,760`、`routes/local_projects/{issues.rs:200,234, projections.rs:240,271,326,338, projects.rs:136,144,167, side.rs:319,355, statuses.rs:149,162}`、`routes/workspaces/create.rs:509,522`、`services/issue_flow.rs:59,74`）全部在 `#[cfg(test)]` 里。前端常量在 `packages/web-core/src/shared/lib/local/identity.ts:9-10`。

相关函数签名（改动时以此为准）：

```
crates/db/src/models/issue.rs:322   pub async fn create(pool: &SqlitePool, data: &CreateIssueRequest) -> Result<Issue, IssueError>
crates/db/src/models/issue_side.rs:292  pub async fn create(pool: &SqlitePool, data: &CreateIssueCommentRequest) -> Result<IssueComment, IssueError>
crates/server/src/routes/local_projects/issues.rs:46  pub(crate) async fn handle_create(pool, payload: CreateIssueRequest) -> Result<Json<TxidResponse>, ApiError>
crates/server/src/routes/local_projects/side.rs:144   pub(crate) async fn handle_comment_create(pool, payload: CreateIssueCommentRequest) -> Result<Json<TxidResponse>, ApiError>
crates/server/src/routes/local_projects/projections.rs:79  pub(crate) async fn handle_workspaces(pool, project_id: Uuid) -> Result<Json<Value>, ApiError>
```

### 2.6 路由注册与契约测试

`crates/server/src/routes/local_projects/mod.rs:43-81` 的 `LocalRoutes` 在挂路由时把「完整路径 + 方法」登记进 `ROUTE_REGISTRY`（:28），`registered_endpoints()`（:35）可读回。契约测试 `前端用到的本地端点都挂上了路由`（:294）用它跟前端 `localEndpoints.ts` / `remoteApi.ts` 对账。**任务 C 的「哪些路由免鉴权」也用同一套思路做测试。**

统一响应辅助：`snapshot()`（:94）、`snapshot_truncatable()`（:100）、`txid()`（:89）、`map_db_error()`（:120）、`map_issue_error()`（:110）。

### 2.7 前端事实

- **入口与数据源开关**：`packages/local-web/src/app/entry/Bootstrap.tsx:81-90`
  ```tsx
  81  // 数据源：显式设置 VITE_VK_DATA_SOURCE 时以它为准；
  82  // 否则没有配置云端基址（VITE_VK_SHARED_API_BASE 为空）就走个人版。
  83  const dataSourceEnv = import.meta.env.VITE_VK_DATA_SOURCE as
  84    | 'local'
  85    | 'remote'
  86    | undefined;
  87  configureDataSource(
  88    dataSourceEnv ??
  89      (import.meta.env.VITE_VK_SHARED_API_BASE ? 'remote' : 'local')
  90  );
  ```
  `VITE_VK_DATA_SOURCE` 在**全仓库只有这一处代码引用**，未在任何 `.env`、`vite.config.ts`、`package.json` 脚本或 `ImportMetaEnv` 里声明。
- `packages/web-core/src/shared/lib/local/dataSource.ts` 全文 16 行：`DataSourceMode = 'local' | 'remote'`，模块级可变单例，默认 `'remote'`，导出 `configureDataSource` / `getDataSourceMode` / `isLocalMode`。
- **本地请求唯一收口**：`packages/web-core/src/shared/lib/localApiTransport.ts:111-116`
  ```ts
  111  export async function makeLocalApiRequest(
  112    pathOrUrl: string,
  113    init: LocalApiRequestOptions = {}
  114  ): Promise<Response> {
  115    return transport.request(resolveScopedPath(pathOrUrl, init), init);
  116  }
  ```
  默认 transport 在 :91-103，`return fetch(pathOrUrl, requestInit);`（:99）——**不设任何请求头、不设 credentials、不算 base URL**。
  调用方：`api.ts:133`（本地 REST，:128-138 只设 `Content-Type`）、`remoteApi.ts:117`（`makeDataRequest` 的 local 分支）、`localCollections.ts`、`useWorkspaces.ts` 等。
- **同源 fetch 默认 `credentials: 'same-origin'`，本来就会带 Cookie**；设计文档说「改成 `credentials: 'include'` 即可」并不是让 Cookie 生效的必要条件（见 §3.3）。
- **实时推送用的是 WebSocket，不是 SSE**：`packages/web-core/src/shared/lib/local/localCollections.ts:288-321` 里 `openLocalApiWebSocket(endpoint.wsPath)`；WS 路径常量在 `localEndpoints.ts:4` `ISSUE_STREAM_PATH = '/api/issues/streams/ws'`。**全仓库 TS 代码里 `EventSource` 出现 0 次，`/api/events` 出现 0 次。**
- `useAuth()`：`packages/web-core/src/shared/hooks/auth/useAuth.ts` 全文 30 行，其中
  ```ts
  if (isLocalMode()) {
    return { isSignedIn: true, isLoaded: true, userId: LOCAL_USER_ID };
  }
  ```
  即本地模式恒为「已登录」。`LocalAuthProvider.tsx`（28 行）在 remote 模式下由 `useUserSystem().loginStatus` 推导。
- 已有的 `oauthApi`（`packages/web-core/src/shared/lib/api.ts:1246-1312`）打的是**云端** `/api/auth/*` 代理路由，与本期无关，不要复用。
- 用户菜单已存在：容器 `packages/web-core/src/shared/components/ui-new/containers/AppBarUserPopoverContainer.tsx`（70 行），视图 `packages/ui/src/components/AppBarUserPopover.tsx`（167 行，props 在 :25-39）。

### 2.8 前端测试与门禁（硬约束）

- **Vitest 只在 `packages/web-core` 配置**：`packages/web-core/vitest.config.ts` 全文 14 行，`environment: 'node'`（:12）、`include: ['src/**/*.test.ts']`（:13）、别名 `@/` → `packages/web-core/src`、`shared` → 仓库 `shared/`。
- 所以：**测试文件必须是 `packages/web-core/src/**/*.test.ts`（不是 `.tsx`），不能渲染 React**（无 jsdom、无 `@testing-library/*`，全仓库均未安装）。
- 现有 4 个测试文件：`shared/lib/diffDataAdapter.test.ts`、`shared/lib/local/localCollections.test.ts`、`shared/lib/local/localEndpoints.test.ts`、`shared/lib/electric/rows.test.ts`。风格：具名 `import { describe, expect, it } from 'vitest'`，中文用例名，纯函数断言。
- 根 `package.json:14` 的 `check` 末尾是 `pnpm run web-core:test`，所以新 vitest 用例被 `pnpm run check` 门禁覆盖。**但 CI 的 `frontend-checks` job（`.github/workflows/test.yml:178-195`）不跑 vitest**，任务 J 要补。
- `pnpm run lint`（`package.json:12`）= local-web eslint + ui eslint + `cargo clippy` + `node scripts/check-unused-i18n-keys.mjs`。**web-core 不过 eslint。**
- **i18n 双重门禁**：
  - `scripts/check-unused-i18n-keys.mjs`（`pnpm lint` 内）对 `packages/web-core/src/i18n/locales/en` 下 5 个 namespace（common / settings / projects / tasks / organization）的每个叶子 key 做「是否在源码里被引用」检查，**没被引用就报错**。
  - `scripts/check-i18n.sh`（CI `frontend-checks`）的 `check_key_consistency`（:91-177，调用点 :231）要求 **7 个语言目录（en / es / fr / ja / ko / zh-Hans / zh-Hant）的 key 集合与 en 完全一致**，缺 key 直接失败。→ **新增任何一个 i18n key，必须同时写进 7 个语言文件。**
  - 同脚本还比对 PR 与 main 分支上 `i18next/no-literal-string` 的违规数（只 lint `packages/local-web`），新增硬编码文案会失败。
- **遗留路径守卫** `scripts/check-legacy-frontend-paths.sh`（`pnpm run check` 第一步）：
  - 禁止在 `packages/local-web/src/components/ui-new`、`packages/local-web/src/components/dialogs` 下新增文件（允许清单 `scripts/legacy-frontend-paths-allowlist.txt` 目前为空）。
  - 禁止 `packages/web-core/src` 里出现 `navigate({ to: '.' ... })`、`appNavigation.navigate(...)`、`@/shared/lib/routes/pathResolution` 导入。
  - 禁止 4 个导航模块出现显式 `any`。
- `packages/local-web/.eslintrc.cjs` 的分层规则：`src/{components,constants,contexts,hooks,keyboard,lib,types,utils}/**` 整个目录被 `no-restricted-syntax` 禁掉（见 :459-472 的 `legacyDirectoryFilePatterns` override），`.tsx` 文件名必须 PascalCase。

### 2.9 本机 HTTP 客户端（会被 Origin 收紧误伤）

`crates/mcp`（二进制 `vibe-kanban-mcp`，被编码智能体调用）用 `reqwest::Client::new()`（`crates/mcp/src/task_server/mod.rs:61,71`）直接打本机 `/api/*`，其中有 POST / PATCH / DELETE（如 `tools/remote_issues.rs:309,548,574`、`tools/sessions.rs:183`、`tools/workspaces.rs:234`），**不带 Origin、不带 Cookie**。base URL 来自 `VIBE_BACKEND_URL` 或端口文件（`crates/mcp/src/bin/vibe_kanban_mcp.rs:100-134`，端口文件 `std::env::temp_dir()/vibe-kanban/vibe-kanban.port`，`crates/utils/src/port_file.rs:18-19`）。`crates/mcp/Cargo.toml:18` 已依赖 `utils`。

`crates/review`、`crates/tauri-app`、`crates/desktop-bridge` 里都没有 `/api/` 字面量（已 grep 确认），只有 mcp 一个外部调用方。

### 2.10 其它

- 监听地址：`crates/server/src/main.rs:115` `let host = std::env::var("HOST").unwrap_or_else(|_| "127.0.0.1".to_string());`
- 前端 dist 编译期嵌入：`crates/server/src/routes/frontend.rs:9-11`（`#[derive(RustEmbed)] #[folder = "../../packages/local-web/dist"]`）。
- 数据目录：`crates/utils/src/assets.rs:6-22`（debug 下是 `<repo>/dev_assets`，release 下是 `ProjectDirs::from("ai","bloop","vibe-kanban").data_dir()`）。
- 已有的 `ApiError` 变体：`Unauthorized`（`crates/server/src/error.rs:73`）、`Forbidden(String)`（:81）、`TooManyRequests(String)`（:83）、`Conflict(String)`（:79）、`BadRequest(String)`（:77）——映射见 `error.rs:459-479`。**新代码直接用，不要新增变体。**
- `Deployment` trait 在 `crates/deployment/src/lib.rs:77-158`，唯一实现是 `local_deployment::LocalDeployment`（`crates/server/src/lib.rs:11`）。
- `local-build.sh:46-47` 硬编码 `VK_SHARED_API_BASE` / `VITE_VK_SHARED_API_BASE` 为 `https://api.vibekanban.com`。
- `Cargo.lock` 中**已有** `subtle 2.6.1`、`rand 0.8.5`、`sha2`、`cookie 0.18.1`；**没有** `argon2`、`password-hash`、`axum-extra`、`wiremock`。
- 状态阶段枚举：`crates/db/src/models/local_project_status.rs:26-32` `StageType { Backlog, Todo, Dev, Review, Done }`，字符串映射 :34-44；默认状态列 `crates/db/src/models/local_project.rs:14-20` 是 `[(&str,&str,&str); 5]`；`stage_type` 在 `shared/remote-types.ts:19` 的 `ProjectStatus` 里**不存在**，且在整个前端源码里出现 0 次。

---

## 3. 对设计文档的纠正（这些地方设计文档写错了或写漏了，必须按本节执行）

### 3.1 `/api/auth/*` 已被占用，会导致启动 panic

`crates/server/src/routes/oauth.rs:81-92` 已经注册了 `/auth/methods`、`/auth/handoff/init`、`/auth/handoff/complete`、`/auth/local/login`、`/auth/logout`、`/auth/status`、`/auth/token`、`/auth/user`（挂在 `/api` 下）。设计文档 §4 写的 `/api/auth/*`（尤其 `/api/auth/logout`）会与之**路径完全重合**，axum 0.8 在 `Router::route` 注册重复路径时直接 panic。

→ **本地账号体系一律用 `/api/local-auth/*`**，云端 `/api/auth/*` 保持原样不动。

### 3.2 `SameSite=Strict` 会打断 OAuth 回调

设计文档 §6.2 写 `SameSite=Strict`。但 OAuth 回调是**从第三方站点发起的跨站顶层导航**到 `/api/local-auth/oauth/{provider}/callback`；浏览器在「跨站重定向链的后续同站请求」上不会带 Strict Cookie，登录会表现为「回调成功但页面仍未登录」。

→ 用 **`SameSite=Lax`**，并用另外两道防线补 CSRF：
1. 写方法（POST/PUT/PATCH/DELETE）+ 带 Cookie ⇒ **必须**有合法 Origin（任务 C1）；
2. 双提交令牌 `vk_csrf` / `X-VK-CSRF`（任务 C2）。

Lax 对跨站 POST 不放行，所以「跨站表单提交」这类经典 CSRF 本来就打不进来；剩下的风险是「跨站顶层 GET 导航」，而本仓库 `/api/*` 的写操作全部是非 GET（任务 C 的步骤 C1.1 会用 `registered_endpoints()` 写一条测试把这个不变量钉住）。

### 3.3 `credentials: 'include'` 不是必需项，真正要改的是 CSRF 头

同源 `fetch` 默认 `credentials: 'same-origin'`，本来就会带 Cookie；`localApiTransport.ts:99` 的裸 `fetch` 在浏览器直连后端时是同源的。真正必须加的是 `X-VK-CSRF` 头。为了显式和为 remote-web 的 WebRTC transport 留余地，仍然在 `makeLocalApiRequest` 里显式写 `credentials: 'same-origin'`（**不是** `'include'`，`'include'` 在跨源时会触发 CORS 预检而本服务端未配 CORS）。

### 3.4 前端不消费 SSE，实时通道是 WebSocket，且存在 CSWSH 风险

设计文档 §2 把实时推送写成 SSE。实测（§2.7）前端只用 `/api/issues/streams/ws`。浏览器**不对 WebSocket 施加同源策略**，握手会带上 Cookie 和 Origin，服务端不校验 Origin 就是跨站 WebSocket 劫持（CSWSH）。

→ 任务 C1 必须额外规定：**带 `Upgrade: websocket` 且带 Cookie 的 GET 请求，Origin 缺失或不匹配一律 403**，并有对应测试。

### 3.5 MCP 在 team 模式下会被打死

§2.9：`vibe-kanban-mcp` 不带 Cookie 也不带 Origin。若只按设计文档做「team 模式下 `/api/*` 全部要求会话」，编码智能体的 MCP 工具会全线 401。

→ 任务 C5 增加**本机令牌**：`asset_dir()/machine_token`（0600，首次启动生成 32 字节随机 base64url），MCP 启动时读取并作为默认请求头 `X-VK-MACHINE-TOKEN` 发送；服务端在 team 模式下把它当作「本机所有者」身份（映射到 `DEFAULT_USER_ID` 对应的 `local_users` 行，该行必须是 `active`，否则 401）。个人版不校验。

### 3.6 relay（云端远程访问）在 team 模式下关闭

带 `X-Vibe-Relay: 1` 的请求会跳过 Origin 校验（`origin.rs:44-46`），其中只有 `relay_signed_routes` 组会被 ed25519 签名校验；且签名中间件的执行顺序在会话中间件**之内**，会话中间件看不到验证结果。

→ 本期采取最简单的 fail-closed 规则：**team 模式下，任何带 `X-Vibe-Relay: 1` 的请求一律 401**（personal 模式行为完全不变）。文档里明确写「团队模式不支持云端 relay 远程访问」。

### 3.7 `DEFAULT_STATUSES` 从 5 列变 6 列会打断既有断言

设计文档 §7.2 要新增 `test` 阶段。`crates/db/src/models/local_project.rs:14` 的类型是 `[(&str, &str, &str); 5]`，且 `crates/db/src/models/local_project_status.rs:304-305` 断言 `statuses[0].stage_type == "backlog"`、`statuses[4].stage_type == "done"`。改成 6 列时这两条断言必须同步改（任务 H1）。

### 3.8 取客户端 IP 需要改 make service 类型

限速要按 IP 分桶，但 `crates/server/src/routes/mod.rs:89` 用的是 `.into_make_service()`，**没有 `ConnectInfo`**，handler 里拿不到对端地址。任务 C3 需要把它换成 `.into_make_service_with_connect_info::<std::net::SocketAddr>()`，并同步改返回类型与两个调用点 `crates/server/src/main.rs:142`、`crates/server/src/startup.rs:54`。

### 3.9 「约十处 DEFAULT_USER_ID 引用」实际只有 4 处生产代码

见 §2.5。任务 B 只需要改那 4 处（外加它们的调用链），其余都是测试里的构造数据。

---

## 4. 文件清单

### 4.1 新建（后端）

| 文件 | 职责 | 任务 |
|---|---|---|
| `crates/db/migrations/20260917000000_add_local_auth.sql` | 4 张新表 + 固定本机用户 | A2 |
| `crates/db/migrations/20260917010000_add_test_stage_status.sql` | 给既有项目补「测试中」状态列 | H1 |
| `crates/db/src/models/local_user.rs` | `LocalUser` / `LocalUserRole` / `LocalUserStatus` / `LocalUsers` | A3 |
| `crates/db/src/models/local_auth.rs` | `LocalSessions` / `LocalUserIdentities` / `LocalInvites` | A4、D、E |
| `crates/services/src/services/server_settings.rs` | `ServerMode` / `ServerSettings` / `load_server_settings` | A1 |
| `crates/services/src/services/local_auth/mod.rs` | 子模块聚合 | A5 |
| `crates/services/src/services/local_auth/password.rs` | Argon2id 哈希与校验 | A5 |
| `crates/services/src/services/local_auth/token.rs` | 令牌生成/哈希、Cookie 构造与解析 | A6 |
| `crates/services/src/services/local_auth/rate_limit.rs` | 登录限速 | C3 |
| `crates/services/src/services/local_auth/runtime.rs` | `LocalAuthRuntime`（设置 + 限速器 + OAuth state + setup token + 机器令牌） | A7 |
| `crates/services/src/services/local_auth/oauth.rs` | OAuth2 抽象与三份预置配置 | E1 |
| `crates/server/src/middleware/local_session.rs` | `require_local_session` + `CurrentUser` | A8 |
| `crates/server/src/routes/local_auth/mod.rs` | `/api/local-auth` 路由树 + DTO | A9 |
| `crates/server/src/routes/local_auth/password_routes.rs` | login / logout / me / password | A9 |
| `crates/server/src/routes/local_auth/setup.rs` | 首启初始化向导 | D4 |
| `crates/server/src/routes/local_auth/oauth_routes.rs` | start / callback / bind | E3 |
| `crates/server/src/routes/admin/mod.rs` | `/api/admin` 路由树 | D2 |
| `crates/server/src/routes/admin/users.rs` | 成员管理 | D2 |
| `crates/server/src/routes/admin/invites.rs` | 邀请码 | D3 |
| `crates/server/tests/mock_oidc.rs`（或 `crates/services/src/services/local_auth/mock_idp.rs`，`#[cfg(test)]`） | 本地 mock OIDC 服务 | E4 |

### 4.2 修改（后端）

| 文件:行 | 改动 | 任务 |
|---|---|---|
| `crates/db/Cargo.toml` | 无需改（`tempfile` 已是常规依赖，:24） | — |
| `crates/db/src/models/mod.rs:9` 之后 | 注册 `local_auth`、`local_user` | A3 |
| `crates/db/src/models/issue.rs:322,429` | `create` 增加 `creator_user_id: Uuid` 参数 | B2 |
| `crates/db/src/models/issue_side.rs:292,332` | `create` 增加 `author_id: Uuid` 参数 | B3 |
| `crates/db/src/models/local_project.rs:14-20` | `DEFAULT_STATUSES` 5 → 6 | H1 |
| `crates/db/src/models/local_project_status.rs:26-44,304-305` | `StageType::Test` + 测试同步 | H1 |
| `crates/db/src/lib.rs:77-86,102-108,136-165` | WAL 开关 + `max_connections` | F1、F2 |
| `crates/services/Cargo.toml` | 新增 `argon2`、`subtle`、`rand`、`sha2`、`base64` | A5 |
| `crates/services/src/services/mod.rs` | 注册 `server_settings`、`local_auth` | A1 |
| `crates/deployment/src/lib.rs:77-158` | trait 新增 `fn local_auth(&self) -> &LocalAuthRuntime` | A7 |
| `crates/local-deployment/src/lib.rs:50-81,88-147` | 字段 + 构造 + `local_auth()` 实现 + WAL 参数 | A7、F1 |
| `crates/server/src/middleware/mod.rs:1-10` | 注册并导出 `local_session` | A8 |
| `crates/server/src/middleware/origin.rs:41-82` | 写方法 + Cookie 强制 Origin；WS 升级强制 Origin | C1 |
| `crates/server/src/routes/mod.rs:38-90` | 挂 `local_auth` / `admin`；会话层；`/health` 移出受保护集合；`into_make_service_with_connect_info` | A9、C |
| `crates/server/src/main.rs:142` / `startup.rs:54` | 适配新的 make service 类型 | C3 |
| `crates/server/src/routes/local_projects/issues.rs:46,143` | 传入当前用户 | B4 |
| `crates/server/src/routes/local_projects/side.rs:144` | 传入当前用户 | B4 |
| `crates/server/src/routes/local_projects/projections.rs:7,115,79` | `owner_user_id` 用当前用户 | B5 |
| `crates/server/src/bin/generate_types.rs:105` 附近 | 注册新 DTO | A9、D、E、G |
| `crates/server/Cargo.toml` | 新增 `subtle`；`argon2` 由 services 提供 | C2 |
| `crates/mcp/src/task_server/mod.rs:59-77` | 读机器令牌并设默认头 | C5 |
| `crates/utils/src/assets.rs:31-52` | 新增 `server_settings_path()`、`machine_token_path()` | A1、C5 |

### 4.3 新建（前端）

| 文件 | 职责 | 任务 |
|---|---|---|
| `packages/web-core/src/shared/lib/local/runtimeMode.ts` | 运行时模式单例（替代构建期 `VITE_VK_DATA_SOURCE`） | G1 |
| `packages/web-core/src/shared/lib/local/runtimeMode.test.ts` | 单测 | G1 |
| `packages/web-core/src/shared/lib/local/csrf.ts` | 从 `document.cookie` 读 `vk_csrf` | G2 |
| `packages/web-core/src/shared/lib/local/csrf.test.ts` | 单测（含畸形 cookie） | G2 |
| `packages/web-core/src/shared/lib/local/bootstrapApi.ts` | `fetchBootstrap()` / `login()` / `logout()` / `fetchMe()` | G3 |
| `packages/web-core/src/shared/lib/local/bootstrapApi.test.ts` | 单测（mock fetch） | G3 |
| `packages/web-core/src/shared/providers/auth/LocalSessionProvider.tsx` | team 模式的会话上下文 | G4 |
| `packages/web-core/src/features/local-auth/ui/LoginPageContainer.tsx` | 登录页容器 | G5 |
| `packages/ui/src/components/LoginPanel.tsx` | 登录页视图（无状态） | G5 |
| `packages/web-core/src/features/local-auth/ui/MembersPageContainer.tsx` | 成员管理容器 | G7 |
| `packages/ui/src/components/MembersPanel.tsx` | 成员管理视图 | G7 |
| `packages/local-web/src/routes/login.tsx` | `/login` 路由 | G5 |
| `packages/local-web/src/routes/_app.members.tsx` | `/members` 路由 | G7 |
| `packages/web-core/src/features/kanban/model/filterIssues.ts` | 从 `useKanbanFilters` 抽出的纯函数 | H3 |
| `packages/web-core/src/features/kanban/model/filterIssues.test.ts` | 单测 | H3 |
| `packages/web-core/src/features/kanban/model/kanbanUrlState.ts` | 筛选 ↔ URL query 互转（纯函数） | H5 |
| `packages/web-core/src/features/kanban/model/kanbanUrlState.test.ts` | 单测 | H5 |
| `packages/web-core/src/features/kanban/model/stageType.ts` | `stage_type` 的前端类型与排序（含 `test`） | H2 |
| `packages/web-core/src/features/kanban/model/stageType.test.ts` | 单测 | H2 |
| `packages/ui/src/components/Skeleton.tsx` | 骨架屏基元 | H6 |
| `packages/ui/src/components/KanbanColumnEmptyState.tsx` | 空列引导 | H6 |
| `packages/web-core/src/features/kanban/ui/KanbanColumn.tsx` | 从 `KanbanContainer` 拆出的单列渲染 | H4 |
| `packages/web-core/src/features/kanban/ui/KanbanBoardView.tsx` | 从 `KanbanContainer` 拆出的看板区 | H4 |

### 4.4 新建（部署与文档）

| 文件 | 任务 |
|---|---|
| `deploy/launchd/ai.bloop.vibe-kanban.plist` | I2 |
| `deploy/systemd/vibe-kanban.service` | I3 |
| `deploy/install-macos.sh` / `deploy/install-linux.sh` | I2、I3 |
| `deploy/backup.sh` / `deploy/restore.sh` | I4 |
| `deploy/server.example.json` | I1 |
| `deploy/Caddyfile.selfhost.example` | I5 |
| `docs/self-hosting/team-mode.mdx` | J1 |
| `docs/self-hosting/backup-restore.mdx` | J2 |
| `docs/docs.json:104-108` 导航补两项 | J1 |

---

## 5. 任务依赖图

```
A 后端认证基础
├─> B 身份落地
├─> C 防护加固 ──┐
├─> D 成员管理 ──┤
│   └─> E 第三方登录
└──────────────> G 前端认证接入（需要 A 的 /bootstrap、C 的 CSRF、D 的成员接口）
                 └─> H 界面优化（H1/H2 需要后端 test 阶段，H3~H8 只依赖 G 的模式开关）

F 数据层（WAL + 连接池）  —— 与 A~E 无依赖，可并行
I 部署                    —— 依赖 A（模式）、F（备份用 .backup）
J 文档与门禁              —— 依赖全部
```

建议顺序：**A → F（并行）→ B → C → D → E → G → H → I → J**。

---

## 任务 A：后端认证基础

**目标：** 迁移 + 用户/会话模型 + Argon2 密码 + 模式配置 + `/api/local-auth/{bootstrap,login,logout,me,password}` + 会话中间件 + 请求上下文里的当前用户。
**依赖：** 无。
**完成后应有：** personal 模式行为完全不变；team 模式下未登录访问受保护路由返回 401。

### A1 · 模式与设置（纯函数优先）

**Create:** `crates/services/src/services/server_settings.rs`
**Modify:** `crates/services/src/services/mod.rs`（在已有 `pub mod` 列表里按字母序插入 `pub mod server_settings;`）、`crates/utils/src/assets.rs`（在 `relay_host_credentials_path()`（:51-53）之后新增 `server_settings_path()`）

- [ ] **A1.1 写失败测试**：在 `crates/services/src/services/server_settings.rs` 里先只写 `#[cfg(test)] mod tests`，覆盖：
  - `模式默认是个人版`：`load_server_settings(None, &|_| None).mode == ServerMode::Personal`
  - `环境变量覆盖配置文件`：文件里 `mode = "personal"`，env `VK_MODE=team` → `Team`
  - `非法模式值报错`：`VK_MODE=teams` → `Err(ServerSettingsError::InvalidMode("teams"))`
  - `模式大小写与空格不敏感`：`" TEAM "` → `Team`
  - `OAuth 凭据缺一不可`：只给 `VK_OAUTH_FEISHU_CLIENT_ID` 不给 `_SECRET` → 该 provider 不出现在 `settings.providers` 里（不是半配置状态）
  - `session_ttl_days 非法值报错`：`VK_SESSION_TTL_DAYS=0` → `Err`；`=abc` → `Err`
  - `trust_proxy 只认 1/true`：`"0"`、`"false"`、`"yes"` → false；`"1"`、`"true"`、`"TRUE"` → true
- [ ] **A1.2 跑测试确认失败**
  `cargo test -p services server_settings`
  预期：`error[E0433]: failed to resolve: use of undeclared crate or module` / `cannot find function \`load_server_settings\``（编译失败即算「红」）。
- [ ] **A1.3 写最小实现**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
  #[serde(rename_all = "lowercase")]
  pub enum ServerMode { Personal, Team }

  #[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
  pub struct ServerSettingsFile { /* 全部字段 Option<...>，含 oauth: Option<BTreeMap<String, OAuthCredentialsFile>> */ }

  #[derive(Debug, Clone)]
  pub struct ServerSettings {
      pub mode: ServerMode,
      pub allow_oauth_signup: bool,
      pub session_ttl_days: u32,
      pub sqlite_wal: bool,
      pub trust_proxy: bool,
      pub public_base_url: Option<String>,
      pub providers: std::collections::BTreeMap<String, OAuthCredentials>,
  }

  pub fn load_server_settings(
      file: Option<ServerSettingsFile>,
      env: &dyn Fn(&str) -> Option<String>,
  ) -> Result<ServerSettings, ServerSettingsError>
  ```
  `env` 做成注入的闭包，测试里传 `&|k| map.get(k).cloned()`，生产里传 `&|k| std::env::var(k).ok()`。**不要在实现里直接调 `std::env::var`**，否则测试会互相污染（Rust 测试默认多线程并行）。
  另外提供 `pub async fn read_server_settings_file(path: &Path) -> Option<ServerSettingsFile>`：文件不存在 → `None`；文件存在但 JSON 非法 → **返回 Err 并让启动失败**（不要静默降级，否则改坏配置会悄悄退回 personal 模式，这是 fail-open）。
- [ ] **A1.4 跑通** `cargo test -p services server_settings` → 预期 `test result: ok. 7 passed`。
- [ ] **A1.5 加 assets 路径**：`crates/utils/src/assets.rs` 新增
  ```rust
  pub fn server_settings_path() -> std::path::PathBuf { asset_dir().join("server.json") }
  ```
  跑 `cargo check -p utils` 通过。
- [ ] **A1.6 提交**
  `git add crates/services/src/services/server_settings.rs crates/services/src/services/mod.rs crates/utils/src/assets.rs`
  `git commit -m "团队版：服务端运行模式与 server.json 设置"`

**验收：** `cargo test -p services server_settings` 全绿；`cargo check -p utils -p services` 通过。

### A2 · 认证迁移

**Create:** `crates/db/migrations/20260917000000_add_local_auth.sql`
**Test:** `crates/db/src/test_support.rs` 的 `#[cfg(test)] mod tests` 内新增用例

- [ ] **A2.1 写失败测试**：在 `crates/db/src/test_support.rs` 的测试模块里加：
  - `认证四张表存在且_id_是第零列`：复用已有的 `column_names()`（:32-45），对 `["local_users","local_sessions","local_user_identities","local_invites"]` 断言 `columns[0] == "id"`。
  - `迁移写入固定本机用户`：
    ```rust
    let row: (Vec<u8>, String, String, String) = sqlx::query_as(
        "SELECT id, username, role, status FROM local_users"
    ).fetch_one(pool).await.unwrap();
    assert_eq!(row.0, uuid::Uuid::from_u128(2).as_bytes().to_vec());
    assert_eq!(row.1, "local");
    assert_eq!(row.2, "admin");
    assert_eq!(row.3, "active");
    ```
    （`DEFAULT_USER_ID = Uuid::from_u128(2)`，见 `crates/db/src/models/local_project.rs:11`；sqlx 把 `Uuid` 编码成 16 字节大端 BLOB，所以 SQL 字面量是 `X'00000000000000000000000000000002'`。**执行者先核实**这一编码：可先写一条临时测试 `sqlx::query("INSERT ...").bind(Uuid::from_u128(2))` 再 `SELECT hex(id)` 打印确认，再删掉临时测试。）
  - `用户名唯一且大小写敏感由应用层负责`：连插两条 `username='local'` → 第二条报唯一约束错。
  - `会话外键级联删除`：删 `local_users` 行后 `local_sessions` 对应行消失（`ON DELETE CASCADE`）。**注意 SQLite 默认不开外键**，测试里先 `PRAGMA foreign_keys = ON`；若发现生产连接也没开，把「是否开启外键」记进 §12 待核实清单。
  - `provider_subject_唯一`：同 `(provider, subject)` 插两次 → 唯一约束错。
- [ ] **A2.2 跑测试确认失败**
  `cargo test -p db 认证四张表`
  预期：`表 local_users 不存在`（`assert!(!columns.is_empty())` 失败）。
- [ ] **A2.3 写迁移**：`crates/db/migrations/20260917000000_add_local_auth.sql`，风格照抄 `20260916000000_add_local_issue_tracking.sql`（开头写中文注释说明 id 必须第 0 列）：
  ```sql
  CREATE TABLE local_users (
      id            BLOB PRIMARY KEY NOT NULL,
      username      TEXT NOT NULL,
      display_name  TEXT NOT NULL,
      email         TEXT,
      password_hash TEXT,
      role          TEXT NOT NULL DEFAULT 'member',
      status        TEXT NOT NULL DEFAULT 'active',
      avatar_color  TEXT NOT NULL DEFAULT '#6366f1',
      created_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
      updated_at    TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
      last_login_at TEXT
  );
  CREATE UNIQUE INDEX idx_local_users_username ON local_users(username);
  CREATE UNIQUE INDEX idx_local_users_email ON local_users(email) WHERE email IS NOT NULL;

  CREATE TABLE local_sessions (
      id           BLOB PRIMARY KEY NOT NULL,
      user_id      BLOB NOT NULL REFERENCES local_users(id) ON DELETE CASCADE,
      token_hash   TEXT NOT NULL,
      created_at   TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
      expires_at   TEXT NOT NULL,
      last_seen_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec')),
      user_agent   TEXT,
      ip           TEXT,
      revoked_at   TEXT
  );
  CREATE UNIQUE INDEX idx_local_sessions_token ON local_sessions(token_hash);
  CREATE INDEX idx_local_sessions_user ON local_sessions(user_id);
  CREATE INDEX idx_local_sessions_expires ON local_sessions(expires_at);

  CREATE TABLE local_user_identities (
      id         BLOB PRIMARY KEY NOT NULL,
      user_id    BLOB NOT NULL REFERENCES local_users(id) ON DELETE CASCADE,
      provider   TEXT NOT NULL,
      subject    TEXT NOT NULL,
      email      TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
  );
  CREATE UNIQUE INDEX idx_local_identities_provider_subject
      ON local_user_identities(provider, subject);
  CREATE INDEX idx_local_identities_user ON local_user_identities(user_id);

  CREATE TABLE local_invites (
      id         BLOB PRIMARY KEY NOT NULL,
      code_hash  TEXT NOT NULL,
      role       TEXT NOT NULL DEFAULT 'member',
      created_by BLOB REFERENCES local_users(id) ON DELETE SET NULL,
      expires_at TEXT NOT NULL,
      used_by    BLOB REFERENCES local_users(id) ON DELETE SET NULL,
      used_at    TEXT,
      created_at TEXT NOT NULL DEFAULT (datetime('now', 'subsec'))
  );
  CREATE UNIQUE INDEX idx_local_invites_code ON local_invites(code_hash);

  -- 个人版固定本机用户：与 crates/db/src/models/local_project.rs:11 的
  -- DEFAULT_USER_ID = Uuid::from_u128(2) 完全一致，保证历史数据里的
  -- creator_user_id / author_id 仍能对上一条真实用户行。
  -- 历史数据不改写：本迁移不 UPDATE issues / issue_comments 的任何字段。
  INSERT INTO local_users (id, username, display_name, role, status)
  VALUES (X'00000000000000000000000000000002', 'local', '本机', 'admin', 'active');
  ```
- [ ] **A2.4 跑通** `cargo test -p db 认证四张表 && cargo test -p db 迁移`（后者跑既有的迁移回归测试 `新迁移可以应用在已有旧迁移的库上并且可重复执行`，确认没打断）。
- [ ] **A2.5 重生成 sqlx 离线数据**（本步骤不涉及新宏，但要确认现有数据仍匹配）：
  `pnpm run prepare-db`
  预期输出末尾 `Database preparation complete!`；`git status` 里 `crates/db/.sqlx` 应无变化（本步没新增宏）。
- [ ] **A2.6 提交**
  `git add crates/db/migrations/20260917000000_add_local_auth.sql crates/db/src/test_support.rs`
  `git commit -m "团队版：本地用户/会话/身份/邀请四张表迁移"`

**验收：** `cargo test -p db` 全绿（预期含新增 5 条用例）。

### A3 · `LocalUsers` 模型

**Create:** `crates/db/src/models/local_user.rs`
**Modify:** `crates/db/src/models/mod.rs`（插入 `pub mod local_auth;` 与 `pub mod local_user;`，按字母序放在 `pub mod local_project;` 之前/之后）

- [ ] **A3.1 写失败测试**（放在 `local_user.rs` 底部 `#[cfg(test)] mod tests`，用 `crate::test_support::TestDb`）：
  - `用户名规范化为小写并去空白`：`normalize_username(" Alice ") == "alice"`
  - `用户名字符集受限`：对 `""`、`"a".repeat(65)`、`"ad min"`、`"admin\u{0}"`、`"аdmin"`（首字母是西里尔 U+0430）、`"admin@x"` 全部返回 `Err(LocalUserError::InvalidUsername)`；对 `"a.b-c_1"` 返回 `Ok`
  - `大小写不同的用户名撞唯一约束`：先建 `Alice`，再建 `alice` → `Err(LocalUserError::Conflict(_))`（不是 500）
  - `固定本机用户可按_id_查到`：`LocalUsers::find_by_id(pool, DEFAULT_USER_ID)` 返回 `Some`，`role == LocalUserRole::Admin`
  - `按用户名查找忽略大小写`：`find_by_username(pool, "LOCAL")` 命中
  - `停用用户仍可查到但_status_是_disabled`
  - `列表按用户名排序且不含_password_hash`：`LocalUser` 结构体里**没有** `password_hash` 字段（单独用 `find_password_hash(pool, id) -> Option<String>` 取）——防止密码哈希被 `Serialize` 到响应里
- [ ] **A3.2 跑测试确认失败** `cargo test -p db local_user` → 预期 `error: couldn't read crates/db/src/models/local_user.rs` 或 `cannot find function`。
- [ ] **A3.3 写最小实现**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
  #[serde(rename_all = "lowercase")]
  pub enum LocalUserRole { Admin, Member }

  #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
  #[serde(rename_all = "lowercase")]
  pub enum LocalUserStatus { Active, Disabled }

  #[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
  pub struct LocalUser {
      pub id: Uuid,
      pub username: String,
      pub display_name: String,
      pub email: Option<String>,
      pub role: LocalUserRole,
      pub status: LocalUserStatus,
      pub avatar_color: String,
      pub created_at: DateTime<Utc>,
      pub updated_at: DateTime<Utc>,
      pub last_login_at: Option<DateTime<Utc>>,
  }
  ```
  函数：`normalize_username`、`create`、`find_by_id`、`find_by_username`、`find_all`、`update_profile`、`set_role`、`set_status`、`set_password_hash`、`find_password_hash`、`touch_last_login`。
  **SQL 用 `sqlx::query_as!` 宏**（在 `crates/db` 里，离线数据由 `pnpm run prepare-db` 覆盖）。枚举列在 SQL 里按 `role AS "role!: LocalUserRole"` 取，需要给两个枚举实现 `sqlx::Type`/`Decode`/`Encode`；**若发现 `#[derive(sqlx::Type)]` 对 `TEXT` 枚举不直接可用**，退一步：结构体里用 `String`，另外提供 `LocalUserRole::from_str` / `as_str`，与 `crates/db/src/models/local_project_status.rs:20`（`pub stage_type: String`）+ `:34-44` 的既有做法保持一致。**优先照抄 `local_project_status.rs` 的做法**，它已经被证明能编译。
  唯一约束错误映射：用 `db_retry::is_unique_violation`（`crates/db/src/models/db_retry.rs:26`）→ `LocalUserError::Conflict`。
  写路径包在 `retry_on_busy`（`db_retry.rs:49`）里，与 `Issues::create`（`issue.rs:367`）一致。
- [ ] **A3.4 跑通** `cargo test -p db local_user` → `ok. 7 passed`。
- [ ] **A3.5 重生成离线数据** `pnpm run prepare-db`，预期 `crates/db/.sqlx` 新增若干 json。
- [ ] **A3.6 提交**
  `git add crates/db/src/models/local_user.rs crates/db/src/models/mod.rs crates/db/.sqlx`
  `git commit -m "团队版：LocalUsers 模型与用户名规范化"`

**验收：** `cargo test -p db local_user`；`pnpm run prepare-db:check` 无差异。

### A4 · `LocalSessions` 模型

**Create:** `crates/db/src/models/local_auth.rs`（本步只写会话部分，identities / invites 留到 D、E）

- [ ] **A4.1 写失败测试**：
  - `创建会话后可按令牌哈希查到`
  - `过期会话查不到`：`expires_at` 设为 `Utc::now() - 1h` → `find_valid_by_token_hash` 返回 `None`
  - `已撤销会话查不到`：`revoked_at` 非空 → `None`
  - `停用用户的会话查不到`：`find_valid_by_token_hash` 做 JOIN 并要求 `local_users.status = 'active'`（**在 SQL 层做，不依赖调用方记得检查**）
  - `撤销用户全部会话`：`revoke_all_for_user` 后两条会话都查不到
  - `last_seen_at_节流`：`touch_last_seen(pool, id, now)` 在距上次不足 5 分钟时返回 `Ok(false)` 且不写库（用 `SELECT last_seen_at` 断言未变）
  - `清理过期会话只删过期的`：`delete_expired(pool, now)` 返回删除条数，未过期的还在
- [ ] **A4.2 跑测试确认失败** `cargo test -p db local_auth` → 编译失败。
- [ ] **A4.3 写实现**：`LocalSession { id, user_id, expires_at, last_seen_at }`（**不含 `token_hash`**，避免误序列化）+ `LocalSessions::{create, find_valid_by_token_hash, touch_last_seen, revoke, revoke_all_for_user, delete_expired}`。
  `find_valid_by_token_hash` 的 SQL 形如：
  ```sql
  SELECT s.id AS "id!: Uuid", s.user_id AS "user_id!: Uuid",
         s.expires_at AS "expires_at!: DateTime<Utc>",
         s.last_seen_at AS "last_seen_at!: DateTime<Utc>"
  FROM local_sessions s
  JOIN local_users u ON u.id = s.user_id
  WHERE s.token_hash = $1
    AND s.revoked_at IS NULL
    AND s.expires_at > $2
    AND u.status = 'active'
  ```
- [ ] **A4.4 跑通** `cargo test -p db local_auth` → `ok. 7 passed`。
- [ ] **A4.5** `pnpm run prepare-db`。
- [ ] **A4.6 提交** `git add crates/db/src/models/local_auth.rs crates/db/src/models/mod.rs crates/db/.sqlx` / `git commit -m "团队版：本地会话模型与失效规则"`

**验收：** `cargo test -p db local_auth`。

### A5 · Argon2id 密码

**Create:** `crates/services/src/services/local_auth/mod.rs`、`crates/services/src/services/local_auth/password.rs`
**Modify:** `crates/services/Cargo.toml`

- [ ] **A5.1 加依赖**：`crates/services/Cargo.toml` 的 `[dependencies]` 末尾追加
  ```toml
  argon2 = { version = "0.5", features = ["std"] }
  subtle = "2"
  rand = { version = "0.8", features = ["std"] }
  sha2 = "0.10"
  base64 = "0.22"
  ```
  **`argon2` / `password-hash` 当前不在 `Cargo.lock` 里**（已确认），第一次 `cargo build` 会联网拉取并改动 `Cargo.lock`。若离线，先 `cargo fetch`。
  跑 `cargo check -p services`，预期成功并产生 `Cargo.lock` 变更。
- [ ] **A5.2 写失败测试**（`password.rs` 底部）：
  - `同一密码两次哈希结果不同`（盐随机）
  - `正确密码校验通过`
  - `错误密码校验失败`
  - `空密码被拒绝`：`hash_password("")` → `Err(PasswordError::TooShort)`
  - `密码长度下限 8 上限 1024`：7 字符 → `Err(TooShort)`；1025 字符 → `Err(TooLong)`（**必须有上限**，否则超长密码会让 Argon2 变成 DoS 向量）
  - `畸形哈希串校验返回 false 而不是 panic`：`verify_password("x", "not-a-phc-string")` → `Ok(false)`（或 `Err`，但绝不 panic，也绝不返回 true）
  - `空哈希串校验返回 false`：`verify_password("x", "")` → 非 true
  - `参数是 OWASP 档位`：断言产生的 PHC 串以 `$argon2id$v=19$m=19456,t=2,p=1$` 开头
- [ ] **A5.3 跑测试确认失败** `cargo test -p services local_auth::password` → 编译失败 `cannot find function hash_password`。
- [ ] **A5.4 写实现**（参考签名，**执行者需先核实 argon2 0.5 的确切 API**，本机 `Cargo.lock` 里没有该 crate，无法静态核实；以 `cargo doc -p argon2 --open` 或 docs.rs 上对应版本为准）：
  ```rust
  use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
  use argon2::password_hash::{SaltString, rand_core::OsRng};

  pub const MIN_PASSWORD_LEN: usize = 8;
  pub const MAX_PASSWORD_LEN: usize = 1024;

  fn hasher() -> Argon2<'static> {
      Argon2::new(Algorithm::Argon2id, Version::V0x13,
                  Params::new(19456, 2, 1, None).expect("固定参数必须合法"))
  }

  pub fn hash_password(plain: &str) -> Result<String, PasswordError> { ... }
  pub fn verify_password(plain: &str, phc: &str) -> bool { ... }   // 任何解析失败都返回 false
  ```
  `verify_password` 返回 `bool` 而不是 `Result`，强制调用方无法「把 Err 当成通过」。
- [ ] **A5.5 跑通** `cargo test -p services local_auth::password` → `ok. 8 passed`。
- [ ] **A5.6 提交** `git add crates/services/Cargo.toml Cargo.lock crates/services/src/services/local_auth/ crates/services/src/services/mod.rs` / `git commit -m "团队版：Argon2id 密码哈希"`

**验收：** `cargo test -p services local_auth::password`；`cargo check --workspace` 通过。

### A6 · 会话令牌与 Cookie

**Create:** `crates/services/src/services/local_auth/token.rs`

- [ ] **A6.1 写失败测试**：
  - `令牌是_43_字符的_base64url`：`generate_session_token()` 长度 43（32 字节 base64url 无填充），字符集 `[A-Za-z0-9_-]`
  - `两次生成不同`
  - `哈希是_64_位小写十六进制`且对同一输入稳定
  - `Cookie 串包含全部属性`：`build_session_cookie("abc", 30, false)` == `vk_session=abc; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000`；`secure=true` 时末尾追加 `; Secure`
  - `登出 Cookie 立即过期`：`build_session_clear_cookie(false)` 含 `Max-Age=0`
  - `parse_cookie 基本用例`：`parse_cookie(Some("a=1; vk_session=xyz; b=2"), "vk_session") == Some("xyz")`
  - **对抗用例**：
    - `parse_cookie(Some("vk_session_evil=bad"), "vk_session") == None`（不能前缀匹配）
    - `parse_cookie(Some("xvk_session=bad"), "vk_session") == None`（不能后缀匹配）
    - `parse_cookie(Some("vk_session="), "vk_session") == Some("")`，且上层必须把空串当作「无会话」
    - `parse_cookie(Some("VK_SESSION=x"), "vk_session") == None`（Cookie 名大小写敏感）
    - `parse_cookie(Some("vk_session=a; vk_session=b"), "vk_session") == Some("a")`（取第一个，且要有测试把这个行为钉死；攻击者可通过子域名注入第二个同名 Cookie）
    - `parse_cookie(None, "vk_session") == None`
    - `parse_cookie(Some("   vk_session   =   x   "), "vk_session") == Some("x")`（两侧空白被裁掉）
  - `令牌里不允许出现分号或等号`（否则会被 Cookie 串注入）：对 1000 次生成结果断言 `!t.contains(';') && !t.contains('=')`
- [ ] **A6.2 跑测试确认失败** `cargo test -p services local_auth::token` → 编译失败。
- [ ] **A6.3 写实现**：
  ```rust
  pub const SESSION_COOKIE: &str = "vk_session";
  pub const CSRF_COOKIE: &str = "vk_csrf";
  pub const CSRF_HEADER: &str = "x-vk-csrf";

  pub fn generate_session_token() -> String;              // rand::rngs::OsRng + 32 字节 + URL_SAFE_NO_PAD
  pub fn hash_session_token(token: &str) -> String;       // sha2::Sha256 -> hex 小写
  pub fn build_session_cookie(token: &str, ttl_days: u32, secure: bool) -> String;
  pub fn build_session_clear_cookie(secure: bool) -> String;
  pub fn build_csrf_cookie(token: &str, ttl_days: u32, secure: bool) -> String;
  pub fn parse_cookie(header: Option<&str>, name: &str) -> Option<String>;
  ```
  `parse_cookie` 实现：按 `;` 切分 → 每段 `trim()` → 用 `split_once('=')` → **键 `trim()` 后精确相等**才算命中 → 值 `trim()`。
- [ ] **A6.4 跑通** `cargo test -p services local_auth::token` → `ok. 14 passed`。
- [ ] **A6.5 提交** `git add crates/services/src/services/local_auth/token.rs crates/services/src/services/local_auth/mod.rs` / `git commit -m "团队版：会话令牌与 Cookie 解析"`

**验收：** `cargo test -p services local_auth::token`。

### A7 · `LocalAuthRuntime` 与 Deployment 接线

**Create:** `crates/services/src/services/local_auth/runtime.rs`
**Modify:** `crates/deployment/src/lib.rs`（trait 里紧跟 `fn auth_context(&self) -> &AuthContext;`（:108）之后加一行）、`crates/local-deployment/src/lib.rs`（结构体字段 :50-81 区块、构造 :88-… 区块、impl 区块 :309 附近）

- [ ] **A7.1 写失败测试**（`runtime.rs` 底部）：
  - `个人模式下 runtime 报告 personal`
  - `团队模式下 secure_cookies 由 public_base_url 的协议决定`：`https://...` → true；`http://...` → false；`None` → false
  - `providers 为空时 available_providers 返回空数组`
- [ ] **A7.2 跑测试确认失败** `cargo test -p services local_auth::runtime`。
- [ ] **A7.3 写实现**：
  ```rust
  #[derive(Clone)]
  pub struct LocalAuthRuntime {
      settings: Arc<ServerSettings>,
      login_limiter: Arc<Mutex<LoginRateLimiter>>,   // C3 之前先放一个空壳
      oauth_states: Arc<Mutex<OAuthStateStore>>,      // E 之前先放一个空壳
      setup_token: Arc<RwLock<Option<SetupToken>>>,   // D 之前先放 None
      machine_token: Arc<String>,                     // C5
  }
  impl LocalAuthRuntime {
      pub fn new(settings: ServerSettings, machine_token: String) -> Self;
      pub fn mode(&self) -> ServerMode;
      pub fn settings(&self) -> &ServerSettings;
      pub fn secure_cookies(&self) -> bool;
      pub fn machine_token(&self) -> &str;
  }
  ```
  为避免一次性引入太多未实现结构，`LoginRateLimiter` / `OAuthStateStore` / `SetupToken` 本步可先定义成最小空结构体，由 C3 / D4 / E1 补齐。
- [ ] **A7.4 接进 Deployment**：
  - `crates/deployment/src/lib.rs`：`use services::services::local_auth::runtime::LocalAuthRuntime;`，trait 里加 `fn local_auth(&self) -> &LocalAuthRuntime;`
  - `crates/local-deployment/src/lib.rs`：结构体加字段 `local_auth: LocalAuthRuntime,`；在 `Deployment::new` 里（`load_config_from_file` 之后、建 DB 之前）加载设置：
    ```rust
    let settings_file = services::services::server_settings::read_server_settings_file(
        &utils::assets::server_settings_path()
    ).await?;
    let server_settings = services::services::server_settings::load_server_settings(
        settings_file, &|k| std::env::var(k).ok()
    ).map_err(|e| DeploymentError::Other(anyhow::anyhow!("server.json/环境变量非法: {e}")))?;
    let machine_token = services::services::local_auth::load_or_create_machine_token().await?;  // C5 之前先返回固定空串
    let local_auth = LocalAuthRuntime::new(server_settings, machine_token);
    ```
    impl 块里加 `fn local_auth(&self) -> &LocalAuthRuntime { &self.local_auth }`。
- [ ] **A7.5 跑通** `cargo check --workspace && cargo test -p services local_auth::runtime`。
- [ ] **A7.6 提交** `git add crates/services/src/services/local_auth/runtime.rs crates/deployment/src/lib.rs crates/local-deployment/src/lib.rs` / `git commit -m "团队版：LocalAuthRuntime 接入 Deployment"`

**验收：** `cargo check --workspace` 通过（这是本步最重要的信号，因为改了 trait）。

### A8 · 会话中间件与 `CurrentUser`

**Create:** `crates/server/src/middleware/local_session.rs`
**Modify:** `crates/server/src/middleware/mod.rs:1-10`（加 `pub mod local_session;` 与 `pub use local_session::*;`）

- [ ] **A8.1 写失败测试**（纯函数部分，不启服务器）：
  ```rust
  pub enum SessionGate { PersonalBypass, RequireSession, Reject(&'static str) }
  pub fn session_gate(mode: ServerMode, is_relay: bool, machine_token_matches: bool) -> SessionGate;
  ```
  用例：
  - `个人模式一律放行`：`session_gate(Personal, false, false) == PersonalBypass`
  - `个人模式即使带 relay 头也放行`：`(Personal, true, false) == PersonalBypass`
  - `团队模式默认要求会话`：`(Team, false, false) == RequireSession`
  - **`团队模式拒绝 relay 请求`**：`(Team, true, false) == Reject("relay_disabled_in_team_mode")` —— 对应 §3.6 的 fail-closed 决定
  - `团队模式的机器令牌走本机所有者身份`：`(Team, false, true) == PersonalBypass`
  - **`relay 头优先于机器令牌`**：`(Team, true, true) == Reject(...)`（攻击者不能靠同时带两个头绕过）
- [ ] **A8.2 跑测试确认失败** `cargo test -p server local_session` → 编译失败。
- [ ] **A8.3 写纯函数实现**，跑通。
- [ ] **A8.4 写中间件**：
  ```rust
  #[derive(Debug, Clone)]
  pub struct CurrentUser {
      pub id: Uuid,
      pub username: String,
      pub role: LocalUserRole,
  }

  pub async fn require_local_session(
      State(deployment): State<DeploymentImpl>,
      mut request: Request,
      next: Next,
  ) -> Result<Response, ApiError>
  ```
  流程：
  1. `let rt = deployment.local_auth();`
  2. 取 `X-Vibe-Relay` 头（常量 `relay_client::RELAY_HEADER`，用法见 `crates/server/src/middleware/origin.rs:99-104`），取 `X-VK-MACHINE-TOKEN` 头，与 `rt.machine_token()` 用 `subtle::ConstantTimeEq` 比较（**空串一律不匹配**）。
  3. `match session_gate(rt.mode(), is_relay, machine_ok)`：
     - `PersonalBypass` → 从库里读 `DEFAULT_USER_ID` 那行；**若该行不存在或 status 不是 active，返回 500 而不是放行**（迁移保证它存在；不存在说明库被改坏了，不能 fail-open）。插入 `CurrentUser` extension，`next.run`。
     - `Reject(reason)` → `tracing::warn!(reason, "拒绝请求")`，返回 `ApiError::Unauthorized`。
     - `RequireSession` → 见 4。
  4. 从 `Cookie` 头里 `parse_cookie(.., SESSION_COOKIE)`；**空值当作无**；`hash_session_token` 后 `LocalSessions::find_valid_by_token_hash`；查不到 → 401。查到 → `LocalUsers::find_by_id`（`None` → 401）→ 插入 `CurrentUser`。
  5. 写方法（`POST/PUT/PATCH/DELETE`）时做 CSRF 双提交校验（任务 C2 补，本步先留 `TODO(C2)` 并在 C2 里补测试）。
  6. `touch_last_seen`（节流 5 分钟）；失败只 `warn`，不影响请求。
- [ ] **A8.5 提取当前用户的辅助**：
  ```rust
  pub fn current_user(request_extensions: &http::Extensions) -> Result<&CurrentUser, ApiError>
  ```
  以及 axum extractor：`impl<S> FromRequestParts<S> for CurrentUser`（从 extensions 克隆；缺失时返回 `ApiError::Unauthorized`）。**缺失时必须报错而不是回退到 `DEFAULT_USER_ID`**，否则任何漏挂中间件的路由都会静默降级成「以本机所有者身份写入」。为此写一条测试：
  - `没有中间件注入时 extractor 报 401`
- [ ] **A8.6 跑通** `cargo test -p server local_session`。
- [ ] **A8.7 提交** `git add crates/server/src/middleware/local_session.rs crates/server/src/middleware/mod.rs` / `git commit -m "团队版：会话中间件与当前用户上下文"`

**验收：** `cargo test -p server local_session`；`cargo check --workspace`。

### A9 · `/api/local-auth` 路由与装配

**Create:** `crates/server/src/routes/local_auth/mod.rs`、`crates/server/src/routes/local_auth/password_routes.rs`
**Modify:** `crates/server/src/routes/mod.rs:9-36`（加 `pub mod local_auth;`）与 `:38-90`（装配）、`crates/server/src/bin/generate_types.rs`

- [ ] **A9.1 写失败测试**（放 `local_auth/mod.rs`）：一条「公共路由清单」测试，思路照抄 `crates/server/src/routes/local_projects/mod.rs:294-312`：
  ```rust
  #[test]
  fn 免鉴权路由清单是白名单而不是黑名单() {
      let expected = vec![
          ("/api/local-auth/bootstrap", "GET"),
          ("/api/local-auth/login", "POST"),
          ("/api/health", "GET"),
      ];
      assert_eq!(public_endpoints(), expected);
  }
  ```
  `public_endpoints()` 由 `public_router()` 在构造时登记（复用 `LocalRoutes` 的登记思路，但换一个独立的 registry，避免污染 `/api/local` 的契约测试）。
  再加：
  - `受保护路由清单不包含任何 local-auth 公共路径`
  - `bootstrap 响应在个人模式下 providers 为空且 require_login 为 false`（对 handler 直接调用，不走 HTTP）
  - **`bootstrap 不泄露任何用户信息`**：断言序列化后的 JSON 里不含 `username`、`email`、`password`
- [ ] **A9.2 跑测试确认失败** `cargo test -p server local_auth`。
- [ ] **A9.3 定义 DTO**（`local_auth/mod.rs`，全部 `#[derive(Serialize, TS)]` 或 `Deserialize`）：
  ```rust
  #[derive(Debug, Serialize, TS)]
  pub struct LocalAuthBootstrap {
      pub mode: String,              // "personal" | "team"
      pub require_login: bool,
      pub authenticated: bool,
      pub needs_setup: bool,         // team 模式且库里没有 active admin
      pub providers: Vec<String>,    // 已配置凭据的 OAuth 提供方 id
      pub allow_oauth_signup: bool,
  }

  #[derive(Debug, Deserialize, TS)]
  pub struct LocalLoginRequest { pub username: String, pub password: String }

  #[derive(Debug, Serialize, TS)]
  pub struct LocalAuthUser {
      pub id: Uuid, pub username: String, pub display_name: String,
      pub email: Option<String>, pub role: String, pub avatar_color: String,
  }

  #[derive(Debug, Deserialize, TS)]
  pub struct ChangePasswordRequest { pub current_password: String, pub new_password: String }
  ```
  响应统一用 `utils::response::ApiResponse`（`crates/utils/src/response.rs:5`），与 `crates/server/src/routes/oauth.rs` 的写法一致。
- [ ] **A9.4 写 handler**（`password_routes.rs`）：
  - `login`：
    - 规范化用户名 → 查用户 → 取 `password_hash` → `verify_password`
    - **用户不存在时也要跑一次 `verify_password` против 一个固定的假哈希**，让成功/失败路径耗时一致（防用户枚举）；测试用例 `不存在的用户与密码错误返回同样的响应体与状态码`
    - 失败：`ApiError::Unauthorized`，响应文案固定 `"用户名或密码错误"`，**不区分两种情况**
    - `status != active` → 同样返回 `"用户名或密码错误"`（不暴露「被停用」）
    - 成功：`LocalSessions::create` → `Set-Cookie: vk_session=...` + `Set-Cookie: vk_csrf=...`（两个 `Set-Cookie` 头，用 `HeaderMap::append`，**不是 `insert`**，否则第二个会覆盖第一个——写一条测试断言响应里有 2 个 `set-cookie`）
    - `touch_last_login`
  - `logout`：撤销当前会话 + 下发两个立即过期的 Cookie。**未登录时也返回 200**（幂等）。
  - `me`：返回 `LocalAuthUser`。
  - `password`：校验 `current_password` → `hash_password(new)` → `set_password_hash` → **`revoke_all_for_user`**（含当前会话）→ 重新建一个会话并下发新 Cookie（否则改完密码自己被登出）。测试：`改密后旧会话失效`、`改密后当前浏览器仍然可用`。
  - `bootstrap`：读 runtime + 查「是否存在 active admin」。
- [ ] **A9.5 装配路由**：改 `crates/server/src/routes/mod.rs:38-90`：
  ```rust
  // 1) /health 从 relay_signed_routes 里删掉（原 :40 那行）
  // 2) relay_signed_routes 追加 .merge(local_auth::protected_router()).merge(admin::router())
  //    —— 放在 .layer(...) 之前，这样它们同样受 relay 签名层保护（顺序与既有一致）
  // 3) 在 api_routes 里：
  let protected = Router::new()
      .merge(relay_auth::router())
      .merge(host_relay::router(&deployment))
      .merge(relay_signed_routes)
      .layer(axum::middleware::from_fn_with_state(
          deployment.clone(),
          middleware::require_local_session,
      ));

  let api_routes = Router::new()
      .route("/health", get(health::health_check))
      .merge(local_auth::public_router())
      .merge(protected)
      .layer(ValidateRequestHeaderLayer::custom(middleware::validate_origin))
      .layer(axum::middleware::from_fn(middleware::log_server_errors))
      .with_state(deployment);
  ```
  **关键：公共路由是「没被 `require_local_session` 包住的那几条」，是结构性 fail-closed，不是路径字符串白名单。** 不要改成在中间件里比对 `request.uri().path()`——在 `.nest("/api", ...)` 内部 `request.uri().path()` 是**去掉 `/api` 前缀**的（证据：`crates/server/src/middleware/error_logging.rs:9-12` 和 `relay_request_signature.rs:184-190` 都必须改用 `OriginalUri` 才能拿到完整路径），很容易写错成放行过宽。
- [ ] **A9.6 注册 TS 类型**：`crates/server/src/bin/generate_types.rs` 在 `server::routes::oauth::CurrentUserResponse::decl(),`（:105）之后插入
  ```rust
  server::routes::local_auth::LocalAuthBootstrap::decl(),
  server::routes::local_auth::LocalLoginRequest::decl(),
  server::routes::local_auth::LocalAuthUser::decl(),
  server::routes::local_auth::ChangePasswordRequest::decl(),
  ```
  跑 `pnpm run generate-types`，确认 `shared/types.ts` 新增 4 个类型。**不要手改 `shared/types.ts`。**
- [ ] **A9.7 跑通**
  `cargo test -p server local_auth && cargo check --workspace && pnpm run generate-types:check`
- [ ] **A9.8 手工冒烟**（可选但强烈建议）：
  ```
  pnpm run dev            # personal 模式
  curl -s localhost:$BACKEND_PORT/api/local-auth/bootstrap | jq
  # 预期 {"success":true,"data":{"mode":"personal","require_login":false,...}}
  curl -s localhost:$BACKEND_PORT/api/local/projects | jq   # 仍然 200
  ```
- [ ] **A9.9 提交**
  `git add crates/server/src/routes/local_auth/ crates/server/src/routes/mod.rs crates/server/src/bin/generate_types.rs shared/types.ts`
  `git commit -m "团队版：/api/local-auth 路由与鉴权装配"`

**验收：**
```
cargo test -p server local_auth        # 预期 ok. 10+ passed
cargo check --workspace                # 预期 Finished
pnpm run generate-types:check          # 预期无 diff
```

---

## 任务 B：身份落地

**目标：** 把 §2.5 列出的 4 处生产代码硬编码换成请求上下文里的当前用户；个人版仍然落 `DEFAULT_USER_ID`；**历史数据一行不改**。
**依赖：** A（需要 `CurrentUser`）。

### B1 · 先钉住「历史数据不改写」

- [ ] **B1.1 写测试**：在 `crates/db/src/test_support.rs` 的测试模块里新增 `认证迁移不改写历史需求的创建人`：
  - 用既有测试 `新迁移可以应用在已有旧迁移的库上并且可重复执行`（:97-193）的套路：先把 migrator 过滤到 `version <= 20260916010000` 跑一遍，插一条 `issues` 行并显式 `creator_user_id = X'...FF'`（一个不存在的用户 id），再跑全量迁移，断言该行的 `creator_user_id` 仍是 `X'...FF'`。
  - 同理对 `issue_comments.author_id` 断言。
- [ ] **B1.2 跑** `cargo test -p db 历史需求的创建人` → 应**直接通过**（迁移里本来就没有 UPDATE）。这条是回归护栏，不是红-绿循环。
- [ ] **B1.3 提交** `git add crates/db/src/test_support.rs` / `git commit -m "团队版：钉住认证迁移不改写历史创建人"`

### B2 · `Issues::create` 接受创建人

**Modify:** `crates/db/src/models/issue.rs:322`（签名）、`:429`（绑定值）、`:928-960` 附近的测试辅助

- [ ] **B2.1 写失败测试**：在 `issue.rs` 的测试模块里新增
  - `创建人取自参数而不是固定用户`：传 `Uuid::from_u128(999)`，断言返回的 `Issue.creator_user_id == Some(Uuid::from_u128(999))`
  - `创建人为默认用户时与旧行为一致`：传 `DEFAULT_USER_ID`，断言等于旧值
- [ ] **B2.2 跑** `cargo test -p db 创建人取自参数` → 预期 `this function takes 2 arguments but 3 arguments were supplied`。
- [ ] **B2.3 改签名**：
  ```rust
  pub async fn create(
      pool: &SqlitePool,
      data: &CreateIssueRequest,
      creator_user_id: Uuid,
  ) -> Result<Issue, IssueError>
  ```
  把 `issue.rs:429` 的 `super::local_project::DEFAULT_USER_ID,` 改成 `creator_user_id,`。
  **注意：这是 `sqlx::query_as!` 宏内的绑定参数，参数类型不变（Uuid），`.sqlx` 缓存的 query hash 由 SQL 文本决定，SQL 文本没变，所以不需要重跑 `prepare-db`。** 仍然在本任务末尾跑一次 `pnpm run prepare-db:check` 确认。
- [ ] **B2.4 修所有调用点**：`cargo check -p db -p server -p services` 会列出全部；已知有
  - `crates/server/src/routes/local_projects/issues.rs:50`（`Issues::create(pool, &payload)`）
  - `crates/db/src/models/issue.rs` 与 `issue_side.rs`、`crates/services/src/services/issue_flow.rs`、`crates/server/src/routes/workspaces/create.rs` 的测试辅助里若干处（传 `DEFAULT_USER_ID` 即可）
- [ ] **B2.5 跑通** `cargo test -p db issue`（预期全绿）。
- [ ] **B2.6 提交** `git add crates/db/src/models/issue.rs crates/server/src/routes/local_projects/issues.rs crates/services/src/services/issue_flow.rs crates/server/src/routes/workspaces/create.rs` / `git commit -m "团队版：需求创建人改由调用方传入"`

### B3 · `IssueComments::create` 接受作者

**Modify:** `crates/db/src/models/issue_side.rs:292`（签名）、`:332`（绑定值）、`:696`（测试断言）

- [ ] **B3.1 写失败测试** `评论作者取自参数`：传 `Uuid::from_u128(888)`，断言 `comment.author_id == Some(...)`。
- [ ] **B3.2 跑确认失败** `cargo test -p db 评论作者取自参数`。
- [ ] **B3.3 改签名** `pub async fn create(pool, data: &CreateIssueCommentRequest, author_id: Uuid)`，:332 的 `DEFAULT_USER_ID` 改成 `author_id`。
- [ ] **B3.4 修调用点**：`crates/server/src/routes/local_projects/side.rs:151`。
- [ ] **B3.5 跑通** `cargo test -p db issue_side`。
- [ ] **B3.6 提交** `git add crates/db/src/models/issue_side.rs crates/server/src/routes/local_projects/side.rs` / `git commit -m "团队版：评论作者改由调用方传入"`

### B4 · 路由层把当前用户透传下去

**Modify:** `crates/server/src/routes/local_projects/issues.rs:46,143`、`crates/server/src/routes/local_projects/side.rs:144` 及对应 router 闭包

- [ ] **B4.1 写失败测试**（放在 `issues.rs` 测试模块）：
  - `建需求时落的是传入的当前用户`：直接调 `handle_create(pool, payload, Uuid::from_u128(7))`，再 `SELECT creator_user_id` 断言。
- [ ] **B4.2 跑确认失败**。
- [ ] **B4.3 实现**：
  ```rust
  pub(crate) async fn handle_create(
      pool: &SqlitePool,
      payload: CreateIssueRequest,
      current_user_id: Uuid,
  ) -> Result<Json<TxidResponse>, ApiError>
  ```
  axum handler 改成：
  ```rust
  async fn create_issue(
      State(deployment): State<DeploymentImpl>,
      current_user: crate::middleware::local_session::CurrentUser,
      Json(payload): Json<CreateIssueRequest>,
  ) -> Result<Json<TxidResponse>, ApiError> {
      handle_create(&deployment.db().pool, payload, current_user.id).await
  }
  ```
  **extractor 顺序：`State` 与自定义 extractor 都实现 `FromRequestParts`，`Json` 必须是最后一个参数**（`FromRequest`）。参照 `crates/server/src/routes/local_projects/projects.rs:88-93` 的既有写法。
  `side.rs` 的评论创建同理（注意它用的是内联闭包写法，见 `side.rs:198-207` 的风格；闭包也能加 extractor 参数）。
- [ ] **B4.4 跑通** `cargo test -p server local_projects`（含既有契约测试 `前端用到的本地端点都挂上了路由`，路径没变所以应仍绿）。
- [ ] **B4.5 提交** `git add crates/server/src/routes/local_projects/issues.rs crates/server/src/routes/local_projects/side.rs` / `git commit -m "团队版：需求与评论写入使用请求上下文用户"`

### B5 · workspaces 投影的 `owner_user_id`

**Modify:** `crates/server/src/routes/local_projects/projections.rs:7,79,115`

- [ ] **B5.1 写失败测试**：既有测试 `projections.rs:326` 断言 `rows[0]["owner_user_id"] == DEFAULT_USER_ID.to_string()`。改成参数化：新增 `工作区投影的_owner_user_id_取自当前用户`，调 `handle_workspaces(pool, project_id, Uuid::from_u128(5))` 并断言。
- [ ] **B5.2 跑确认失败**。
- [ ] **B5.3 实现**：`handle_workspaces(pool, project_id, owner_user_id: Uuid)`，:115 改成 `owner_user_id,`；删掉 :7 的 `use db::models::local_project::DEFAULT_USER_ID;`（若测试仍需要则移进测试模块）。router 闭包加 `CurrentUser` extractor。
  **注意语义：** 本地 workspaces 表没有 owner 列，这里是「投影出一个云端结构要求的字段」。用当前请求者的 id 意味着不同成员看到的 `owner_user_id` 不同。**这是刻意的**：前端 `USER_WORKSPACES_SHAPE` 按 `owner_user_id` 过滤（`packages/web-core/src/shared/lib/local/localEndpoints.ts` 对该 shape 返回 `{kind:'empty'}`，所以实际不受影响）。在代码里写注释说明，并在 §12 待核实清单里留一条。
- [ ] **B5.4 跑通** `cargo test -p server projections`。
- [ ] **B5.5 全量回归** `cargo test --workspace`（预期全绿）。
- [ ] **B5.6 提交** `git add crates/server/src/routes/local_projects/projections.rs` / `git commit -m "团队版：工作区投影的 owner_user_id 使用当前用户"`

**任务 B 验收：**
```
cargo test --workspace                 # 全绿
pnpm run prepare-db:check              # 无差异
```

---

## 任务 C：防护加固

**目标：** Origin 对写方法与 WebSocket 升级强校验、双提交 CSRF、登录限速、会话清理、本机令牌。
**依赖：** A。
**这是本计划安全性最关键的一段，每一步都必须先有攻击样例测试。**

### C1 · Origin 强校验

**Modify:** `crates/server/src/middleware/origin.rs:41-82` 与测试模块 `:154-260`

- [ ] **C1.1 写攻击样例测试**（加在 `origin.rs` 的 `mod tests` 里，扩展 `make_request` 以支持 method / 额外头）：
  | 用例名 | 请求 | 预期 |
  |---|---|---|
  | `带_cookie_的写请求缺少_origin_被拒` | `POST /api/local/issues`，`Host: kanban.lan:8080`，`Cookie: vk_session=x`，无 Origin | 403 |
  | `带_cookie_的写请求_origin_跨站被拒` | 同上 + `Origin: http://evil.example` | 403 |
  | `带_cookie_的写请求同源放行` | 同上 + `Origin: http://kanban.lan:8080` | Ok |
  | `不带_cookie_的写请求缺少_origin_仍放行` | `POST`，无 Cookie，无 Origin | Ok（MCP 场景，见 §2.9） |
  | `带_cookie_的_get_请求缺少_origin_仍放行` | `GET`，`Cookie: vk_session=x`，无 Origin | Ok（顶层导航不带 Origin） |
  | `websocket_升级带_cookie_跨站被拒` | `GET /api/issues/streams/ws`，`Upgrade: websocket`，`Connection: Upgrade`，`Cookie: vk_session=x`，`Origin: http://evil.example` | 403（CSWSH） |
  | `websocket_升级带_cookie_缺少_origin_被拒` | 同上但无 Origin | 403 |
  | `websocket_升级带_cookie_同源放行` | 同上，`Origin: http://kanban.lan:8080` | Ok |
  | `websocket_升级不带_cookie_缺少_origin_放行` | 同上但无 Cookie | Ok |
  | `upgrade_头大小写不敏感` | `Upgrade: WebSocket` | 与小写同样处理 |
  | `origin_为_null_一律被拒` | 已有用例 `null_origin_is_forbidden`（:181） | 保持 |
  | `relay_头仍然整体跳过` | `POST` + `X-Vibe-Relay: 1` + `Origin: http://evil.example` | Ok（由 A8 的会话层在 team 模式拒掉） |
  | `方法大小写由_http_库保证` | 用 `Method::POST` 构造 | — |
  | `PUT/PATCH/DELETE_与_POST_同规则` | 三个方法各一条 | 403 |
- [ ] **C1.2 跑确认失败** `cargo test -p server origin` → 预期 `带_cookie_的写请求缺少_origin_被拒` 失败（当前 `origin.rs:48-50` 直接 `Ok(())`）。
- [ ] **C1.3 实现**：在 `validate_origin` 里，`is_relay_request` 判断之后插入：
  ```rust
  let has_cookie = req.headers().contains_key(header::COOKIE);
  let is_write = matches!(*req.method(), Method::POST | Method::PUT | Method::PATCH | Method::DELETE);
  let is_ws_upgrade = req.headers().get(header::UPGRADE)
      .and_then(|v| v.to_str().ok())
      .is_some_and(|v| v.eq_ignore_ascii_case("websocket"));
  let origin_required = has_cookie && (is_write || is_ws_upgrade);

  let Some(origin) = get_origin_header(req) else {
      return if origin_required { Err(forbidden()) } else { Ok(()) };
  };
  ```
  其余逻辑不变。
  **注意 `validate_origin` 的签名是 `fn validate_origin<B>(req: &mut Request<B>)`，`req.method()` 可用。**
- [ ] **C1.4 跑通** `cargo test -p server origin` → 预期原有 8 条 + 新增 14 条全绿。
- [ ] **C1.5 「没有 GET 写操作」的不变量测试**（对应 §3.2）：在 `crates/server/src/routes/local_projects/mod.rs` 的测试模块新增
  ```rust
  #[test]
  fn 本地写操作都不是_get() {
      let _ = router();
      for (path, methods) in registered_endpoints() {
          if methods.contains("GET") {
              assert!(!path.ends_with("/bulk"), "{path} 不应该用 GET");
          }
      }
  }
  ```
  外加一条人工核对项写进 §12：**执行者必须人工扫一遍 `crates/server/src/routes/` 全部 `get(...)` 路由，确认没有 GET 做写操作**（`registered_endpoints()` 只覆盖 `/api/local`）。
- [ ] **C1.6 提交** `git add crates/server/src/middleware/origin.rs crates/server/src/routes/local_projects/mod.rs` / `git commit -m "加固：写方法与 WebSocket 升级在携带 Cookie 时强制校验 Origin"`

**验收：** `cargo test -p server origin` 全绿。

### C2 · 双提交 CSRF

**Modify:** `crates/server/src/middleware/local_session.rs`（A8.4 第 5 步的 TODO）、`crates/server/Cargo.toml`（加 `subtle = "2"`）

- [ ] **C2.1 写攻击样例测试**（纯函数）：
  ```rust
  pub fn csrf_check(method: &Method, cookie: Option<&str>, header: Option<&str>) -> Result<(), &'static str>;
  ```
  | 用例 | 输入 | 预期 |
  |---|---|---|
  | `读方法不校验` | `GET, None, None` | `Ok` |
  | `写方法两者一致通过` | `POST, Some("abc"), Some("abc")` | `Ok` |
  | `写方法缺头被拒` | `POST, Some("abc"), None` | `Err("csrf_missing_header")` |
  | `写方法缺_cookie_被拒` | `POST, None, Some("abc")` | `Err("csrf_missing_cookie")` |
  | `写方法两者都缺被拒` | `POST, None, None` | `Err(_)` |
  | **`两者都是空串被拒`** | `POST, Some(""), Some("")` | `Err("csrf_empty")` —— 否则攻击者伪造两个空值即可通过 |
  | `不一致被拒` | `POST, Some("abc"), Some("abd")` | `Err("csrf_mismatch")` |
  | `长度不同被拒且不泄露长度` | `POST, Some("abc"), Some("abcd")` | `Err("csrf_mismatch")` |
  | `HEAD/OPTIONS 不校验` | `HEAD/OPTIONS` | `Ok` |
  | `PUT/PATCH/DELETE 校验` | 三条 | `Err` |
- [ ] **C2.2 跑确认失败** `cargo test -p server csrf`。
- [ ] **C2.3 实现**：用 `subtle::ConstantTimeEq`
  ```rust
  use subtle::ConstantTimeEq;
  let ok: bool = cookie.as_bytes().ct_eq(header.as_bytes()).into();
  ```
  长度不同时 `ct_eq` 直接返回 0，这没问题（长度本来就不是秘密）。
- [ ] **C2.4 接进中间件**：只在 `SessionGate::RequireSession` 分支（即 team 模式的真实会话请求）里调用；`PersonalBypass` 不校验（个人版无 Cookie，加了只会误伤 MCP）。失败返回 `ApiError::Forbidden("请求缺少或不匹配 CSRF 令牌".into())`（403，不是 401——401 会让前端误判为会话过期而跳登录页）。
- [ ] **C2.5 测试「中间件层」行为**：用 `axum::body::Body` + `tower::ServiceExt::oneshot` 起一个最小 `Router`（**不需要真实 `DeploymentImpl`**：把中间件里依赖 deployment 的部分抽成 `async fn authenticate(rt: &LocalAuthRuntime, pool: &SqlitePool, parts: &Parts) -> Result<CurrentUser, ApiError>`，在测试里直接调它，用 `TestDb`）。用例：
  - `团队模式无_cookie_访问受保护路由返回_401`
  - `团队模式伪造_cookie_返回_401`：`vk_session=` + 64 个 `a`
  - `团队模式过期会话返回_401`
  - `团队模式被停用用户返回_401`
  - `团队模式合法会话但写方法缺_CSRF_返回_403`
  - `个人模式无_cookie_也能拿到本机用户`
  `tower` 已在 `Cargo.lock`（0.5.3）但不是 `crates/server` 的直接依赖；若要用 `oneshot`，在 `[dev-dependencies]` 加 `tower = { version = "0.5", features = ["util"] }`。**若发现引入 tower 麻烦，就只测抽出来的 `authenticate` 纯异步函数，不测 HTTP 层**——本计划接受后者。
- [ ] **C2.6 跑通** `cargo test -p server local_session`。
- [ ] **C2.7 提交** `git add crates/server/Cargo.toml Cargo.lock crates/server/src/middleware/local_session.rs` / `git commit -m "加固：写操作双提交 CSRF 校验"`

**验收：** `cargo test -p server local_session csrf`。

### C3 · 登录限速（含真实客户端 IP）

**Create:** `crates/services/src/services/local_auth/rate_limit.rs`
**Modify:** `crates/server/src/routes/mod.rs:89`、`crates/server/src/main.rs:142`、`crates/server/src/startup.rs:54`

- [ ] **C3.1 写测试**（纯函数 + 可注入时钟）：
  ```rust
  pub struct LoginRateLimiter { /* HashMap<String, Vec<Instant>> 两张表 */ }
  impl LoginRateLimiter {
      pub fn check(&mut self, username: &str, ip: &str, now: Instant) -> Result<(), Duration>;
      pub fn record_failure(&mut self, username: &str, ip: &str, now: Instant);
      pub fn reset(&mut self, username: &str, ip: &str);
      pub fn prune(&mut self, now: Instant);
  }
  ```
  用例：
  | 用例 | 预期 |
  |---|---|
  | `五分钟内十次失败后被拒` | 第 11 次 `check` 返回 `Err(剩余时间)` |
  | `窗口滑出后恢复` | `now + 5min + 1s` 后 `check` 返回 `Ok` |
  | `成功登录清空计数` | `record_failure` × 9 → `reset` → `check` Ok |
  | **`同一 IP 换用户名也被限速`** | 用 30 个不同用户名各失败 1 次（同 IP）→ 第 31 次被拒（防用户枚举喷洒） |
  | **`不同 IP 互不影响`** | IP A 打满后 IP B 仍 Ok |
  | **`IP 未知时共用一个桶而不是放行`** | `ip = ""` 时按固定键 `"unknown"` 计数，打满后被拒（fail-closed） |
  | `用户名大小写归一` | `Alice` 与 `alice` 共用一个桶 |
  | `prune 清掉过期条目` | `prune` 后 `len()` 归零，防内存无限增长 |
  | `退避时间随失败次数增长` | 第 11 次返回的 `Duration` ≥ 第 15 次之前的值（指数退避，上限 15 分钟） |
- [ ] **C3.2 跑确认失败** `cargo test -p services rate_limit`。
- [ ] **C3.3 实现**，跑通。
- [ ] **C3.4 接真实 IP**：
  - `crates/server/src/routes/mod.rs:38` 的返回类型改成
    ```rust
    pub fn router(deployment: DeploymentImpl)
        -> axum::extract::connect_info::IntoMakeServiceWithConnectInfo<Router, std::net::SocketAddr>
    ```
    最后一行 `.into_make_service()` 改成 `.into_make_service_with_connect_info::<std::net::SocketAddr>()`。
  - `crates/server/src/main.rs:142` 与 `crates/server/src/startup.rs:54` 不需要改代码（类型推导），但 `axum::serve(listener, app_router)` 要能接受新类型——**执行者需先核实 axum 0.8 的 `serve` 对 `IntoMakeServiceWithConnectInfo` 的支持**（0.8 支持，但请以 `cargo check` 为准）。
  - 新增纯函数（放 `rate_limit.rs`）：
    ```rust
    pub fn client_ip(trust_proxy: bool, forwarded_for: Option<&str>, peer: Option<std::net::IpAddr>) -> String;
    ```
    测试：
    | 用例 | 预期 |
    |---|---|
    | `不信任代理时忽略_x_forwarded_for` | `client_ip(false, Some("1.2.3.4"), Some(127.0.0.1)) == "127.0.0.1"` |
    | `信任代理时取第一段` | `client_ip(true, Some("1.2.3.4, 5.6.7.8"), _) == "1.2.3.4"` |
    | **`信任代理但头是垃圾时回落到对端`** | `client_ip(true, Some("not-an-ip"), Some(10.0.0.1)) == "10.0.0.1"` |
    | **`信任代理但头为空时回落到对端`** | `client_ip(true, Some(""), Some(10.0.0.1)) == "10.0.0.1"` |
    | `对端也没有时返回 unknown` | `client_ip(false, None, None) == "unknown"` |
- [ ] **C3.5 接进 login handler**：`login` 里先 `check`，被拒则 `ApiError::TooManyRequests("登录尝试过于频繁，请稍后再试".into())`（429，`crates/server/src/error.rs:470-474` 已有映射）；密码错误后 `record_failure`；成功后 `reset`。**限速检查必须在读库之前**，否则打满 CPU 的仍然是 Argon2。
- [ ] **C3.6 跑通** `cargo test -p services rate_limit && cargo check --workspace`。
- [ ] **C3.7 提交** `git add crates/services/src/services/local_auth/rate_limit.rs crates/server/src/routes/mod.rs crates/server/src/routes/local_auth/ crates/server/src/main.rs crates/server/src/startup.rs` / `git commit -m "加固：登录限速与真实客户端 IP"`

**验收：** `cargo test -p services rate_limit`；`cargo check --workspace`。

### C4 · 会话清理任务

**Modify:** `crates/local-deployment/src/lib.rs`（`Deployment::new` 末尾，参考 :149-158 里 `tokio::spawn` 清理孤儿文件的写法）

- [ ] **C4.1 写测试**：`LocalSessions::delete_expired` 已在 A4 测过。本步只加一条 `清理任务的间隔常量是六小时`：`assert_eq!(SESSION_CLEANUP_INTERVAL, Duration::from_secs(6 * 3600));`（防手滑写成 6 秒）。
- [ ] **C4.2 实现**：
  ```rust
  const SESSION_CLEANUP_INTERVAL: Duration = Duration::from_secs(6 * 3600);
  let pool = db.pool.clone();
  let shutdown = shutdown.clone();
  tokio::spawn(async move {
      loop {
          match db::models::local_auth::LocalSessions::delete_expired(&pool, Utc::now()).await {
              Ok(n) if n > 0 => tracing::info!("清理过期会话 {n} 条"),
              Ok(_) => {}
              Err(e) => tracing::warn!("清理过期会话失败: {e}"),
          }
          tokio::select! {
              _ = tokio::time::sleep(SESSION_CLEANUP_INTERVAL) => {}
              _ = shutdown.cancelled() => break,
          }
      }
  });
  ```
  **必须监听 `shutdown`**（`CancellationToken` 已是 `Deployment::new` 的入参，见 `crates/deployment/src/lib.rs:79`），否则进程退出会被这个 loop 拖住。
- [ ] **C4.3 跑通** `cargo check --workspace`。
- [ ] **C4.4 提交** `git add crates/local-deployment/src/lib.rs crates/db/src/models/local_auth.rs` / `git commit -m "加固：启动时与每六小时清理过期会话"`

### C5 · 本机令牌（保住 MCP）

**Create:** `crates/services/src/services/local_auth/machine_token.rs`
**Modify:** `crates/utils/src/assets.rs`、`crates/mcp/src/task_server/mod.rs:59-77`

- [ ] **C5.1 写测试**：
  - `首次调用生成并落盘`：临时目录里调两次 `load_or_create_machine_token_at(path)`，两次结果相同
  - `令牌是_43_字符_base64url`
  - `文件权限是_0600`（`#[cfg(unix)]`，`metadata.permissions().mode() & 0o777 == 0o600`）
  - **`空文件被当作没有令牌并重新生成`**（防止「文件被清空 → 空令牌 → 空头匹配」的 fail-open）
  - **`只含空白的文件同样重新生成`**
- [ ] **C5.2 跑确认失败** `cargo test -p services machine_token`。
- [ ] **C5.3 实现** + `crates/utils/src/assets.rs` 加
  ```rust
  pub fn machine_token_path() -> std::path::PathBuf { asset_dir().join("machine_token") }
  ```
- [ ] **C5.4 服务端比对必须拒绝空串**：在 A8.4 第 2 步的比对里加 `if provided.is_empty() || expected.is_empty() { false }`，并补测试 `空机器令牌不匹配`、`错误机器令牌不匹配`、`正确机器令牌匹配`。
- [ ] **C5.5 改 MCP 客户端**：`crates/mcp/src/task_server/mod.rs` 的 `new_global`（:59）与 `new_orchestrator`（:69）里，把 `reqwest::Client::new()` 换成带默认头的构造：
  ```rust
  fn build_client() -> reqwest::Client {
      let mut headers = reqwest::header::HeaderMap::new();
      if let Some(token) = utils::assets::read_machine_token() {   // 读不到就不加头
          if let Ok(v) = reqwest::header::HeaderValue::from_str(&token) {
              headers.insert("x-vk-machine-token", v);
          }
      }
      reqwest::Client::builder().default_headers(headers).build()
          .unwrap_or_else(|_| reqwest::Client::new())
  }
  ```
  `read_machine_token()` 放在 `crates/utils/src/assets.rs`（同步读，返回 `Option<String>`，空白视为 None）。**mcp 已依赖 utils**（`crates/mcp/Cargo.toml:18`）。
- [ ] **C5.6 跑通** `cargo test -p services machine_token && cargo check --workspace`。
- [ ] **C5.7 手工冒烟**（team 模式）：
  ```
  VK_MODE=team BACKEND_PORT=8080 cargo run --bin server
  curl -i -X POST localhost:8080/api/local/projects -H 'Content-Type: application/json' -d '{}'
  # 预期 401
  curl -s localhost:8080/api/local/projects -H "X-VK-MACHINE-TOKEN: $(cat dev_assets/machine_token)"
  # 预期 200
  ```
- [ ] **C5.8 提交** `git add crates/services/src/services/local_auth/machine_token.rs crates/utils/src/assets.rs crates/mcp/src/task_server/mod.rs crates/server/src/middleware/local_session.rs` / `git commit -m "加固：本机令牌，保证团队模式下 MCP 仍可用"`

**任务 C 验收：**
```
cargo test -p server origin
cargo test -p server local_session
cargo test -p services local_auth
cargo test --workspace
```
外加人工核对：`grep -rn "get(" crates/server/src/routes/ | grep -v "get(list\|get(get\|get(stream\|get(health"` 逐条确认没有 GET 写操作。

---

## 任务 D：成员管理

**目标：** `/api/admin/users/*`、邀请码、首启初始化向导（一次性令牌）。
**依赖：** A、B。

### D1 · `LocalInvites` 模型

**Modify:** `crates/db/src/models/local_auth.rs`（追加）

- [ ] **D1.1 写测试**：
  - `邀请码只存哈希`：`LocalInvite` 结构体没有 `code_hash` 字段以外的明文字段；`create` 返回 `(LocalInvite, String /*明文码*/)`，明文只在创建那一次返回
  - `过期邀请不可用`：`find_usable_by_code_hash` 对 `expires_at < now` 返回 `None`
  - `已使用邀请不可用`：`used_at` 非空 → `None`
  - **`并发使用同一邀请只有一个成功`**：用 `UPDATE local_invites SET used_by=?, used_at=? WHERE id=? AND used_at IS NULL` 的条件更新，断言第二次 `rows_affected() == 0`（不要先查后写）
  - `删除邀请返回受影响行数`
- [ ] **D1.2 跑确认失败** `cargo test -p db local_invite`。
- [ ] **D1.3 实现** + `pnpm run prepare-db`。
- [ ] **D1.4 提交** `git add crates/db/src/models/local_auth.rs crates/db/.sqlx` / `git commit -m "团队版：邀请码模型与一次性消费"`

### D2 · `/api/admin/users`

**Create:** `crates/server/src/routes/admin/mod.rs`、`crates/server/src/routes/admin/users.rs`
**Modify:** `crates/server/src/routes/mod.rs`（`pub mod admin;` + merge 进 `relay_signed_routes`）、`generate_types.rs`

- [ ] **D2.1 写攻击样例测试**（handler 级，直接调纯函数版本，避免起 HTTP）：
  抽出 `pub(crate) async fn handle_list_users(pool, actor: &CurrentUser)` 等，测试：
  | 用例 | 预期 |
  |---|---|
  | `member 调用列表返回_403` | `ApiError::Forbidden(_)` |
  | `admin 调用列表成功` | `Ok` |
  | `member 调用建号返回_403` | `Forbidden` |
  | **`admin 不能把自己降级成 member`** | `Forbidden("不能修改自己的角色")`——否则最后一个管理员可以把自己降权，系统再无管理员 |
  | **`不能停用最后一个 active admin`** | `Conflict("至少保留一个可用管理员")`（用 `SELECT COUNT(*) FROM local_users WHERE role='admin' AND status='active' AND id != ?` 判断） |
  | **`不能停用自己`** | `Forbidden` |
  | `建号时用户名冲突返回_409` | `Conflict` |
  | `建号时弱密码返回_400` | 7 字符 → `BadRequest` |
  | **`响应里不含 password_hash`** | 序列化后 JSON 不含 `password` 子串 |
  | `重置他人密码会撤销该用户全部会话` | 重置后 `find_valid_by_token_hash` 返回 `None` |
  | **`重置他人密码不会撤销操作者自己的会话`** | 操作者会话仍有效 |
  | `停用用户后其会话立即失效` | `find_valid_by_token_hash` 因 JOIN 条件返回 `None`（A4 已保证，这里做端到端断言） |
  | `personal 模式下 admin 路由仍可用` | 个人版的固定用户是 admin，返回 `Ok`（不特殊处理，简单一致） |
- [ ] **D2.2 跑确认失败** `cargo test -p server admin`。
- [ ] **D2.3 实现**：
  - DTO：`CreateLocalUserRequest { username, display_name, email: Option<String>, password: String, role: String }`、`UpdateLocalUserRequest { display_name: Option<String>, email: Option<Option<String>>, role: Option<String>, status: Option<String> }`、`ResetPasswordRequest { new_password: String }`、`ListLocalUsersResponse { users: Vec<LocalAuthUser> }`。
  - 统一的守卫：`fn require_admin(actor: &CurrentUser) -> Result<(), ApiError>`，**每个 handler 第一行调用**。写一条测试 `每个_admin_handler_都调用了_require_admin`（用 `include_str!(file!())` 读自身源码 + 计数断言；若觉得脆弱，退而求其次：把所有 admin 路由挂在一个专用子 Router 上，再加一层 `require_admin_middleware`，并用路由登记表测试断言「`/api/admin` 前缀下的路径全部在该子 Router 里」）。**本计划推荐后者（中间件），因为它是结构性的。**
- [ ] **D2.4 跑通 + 注册 TS 类型 + `pnpm run generate-types`**。
- [ ] **D2.5 提交** `git add crates/server/src/routes/admin/ crates/server/src/routes/mod.rs crates/server/src/bin/generate_types.rs shared/types.ts` / `git commit -m "团队版：成员管理接口"`

### D3 · 邀请码接口

**Create:** `crates/server/src/routes/admin/invites.rs`；注册接口放 `crates/server/src/routes/local_auth/mod.rs`

- [ ] **D3.1 写攻击样例测试**：
  | 用例 | 预期 |
  |---|---|
  | `member 创建邀请返回_403` | `Forbidden` |
  | `邀请码明文只在创建响应里出现一次` | 列表接口返回的 `LocalInviteInfo` 不含明文 |
  | `使用不存在的邀请码返回_400 且文案统一` | `BadRequest("邀请码无效或已过期")` |
  | `使用过期邀请码返回同样文案` | 同上（不区分，防枚举） |
  | `使用已消费邀请码返回同样文案` | 同上 |
  | **`同一邀请码并发注册只有一个成功`** | 第二次 `BadRequest` |
  | `注册时用户名冲突返回_409 且邀请码未被消费` | 邀请仍可用（**必须在事务里：先占用户名再消费邀请**，或者失败时回滚） |
  | `注册产生的角色来自邀请而不是请求体` | 请求体里传 `role=admin` 被忽略，实际是邀请里的 `member` |
  | `注册接口在_personal_模式下返回_404` | 个人版不开放自助注册 |
  | `邀请码默认有效期 7 天` | 常量断言 |
- [ ] **D3.2 跑确认失败**。
- [ ] **D3.3 实现**：邀请码 = 32 字节 base64url 明文，库里存 SHA-256（复用 `hash_session_token`，或另起 `hash_invite_code` 但用同一实现）。`POST /api/local-auth/invites/accept` 请求体 `{ code, username, display_name, password }`。
  **整个消费流程必须在一个 sqlx 事务里**：`UPDATE local_invites SET used_by=?, used_at=? WHERE id=? AND used_at IS NULL` → `rows_affected() == 1` 才继续 → `INSERT INTO local_users` → 冲突则整体回滚。
- [ ] **D3.4 跑通 + 提交** `git add crates/server/src/routes/admin/invites.rs crates/server/src/routes/local_auth/mod.rs crates/db/src/models/local_auth.rs shared/types.ts` / `git commit -m "团队版：邀请码创建与注册"`

### D4 · 首启初始化向导

**Create:** `crates/server/src/routes/local_auth/setup.rs`
**Modify:** `crates/services/src/services/local_auth/runtime.rs`（`SetupToken`）、`crates/server/src/main.rs`（启动时打印链接）

- [ ] **D4.1 写攻击样例测试**：
  | 用例 | 预期 |
  |---|---|
  | `库里已有 active admin 时 setup 返回_409` | `Conflict("已初始化")`——**这是最关键的一条，否则任何人都能再建一个管理员** |
  | `令牌错误返回_401` | 固定文案 |
  | `令牌缺失返回_401` | |
  | `令牌为空串返回_401` | 防 fail-open |
  | **`令牌比对是常量时间`** | 用 `subtle::ConstantTimeEq`（代码审查项，测试只断言函数被调用/不用 `==`） |
  | `令牌 30 分钟后过期` | 注入时钟 |
  | **`令牌用后立即失效`** | 第二次同样令牌 → 401（即使 30 分钟没到） |
  | `personal 模式下 setup 返回_404` | 个人版不需要向导 |
  | `弱密码被拒` | `BadRequest` |
  | `创建的第一个用户角色强制是_admin` | 忽略请求体里的 role |
  | **`并发两次 setup 只有一个成功`** | 第二次 `Conflict`（令牌一次性 + admin 存在性检查双保险） |
- [ ] **D4.2 跑确认失败** `cargo test -p server setup`。
- [ ] **D4.3 实现**：
  - `SetupToken { value: String, expires_at: Instant }` 放在 `LocalAuthRuntime` 的 `Arc<RwLock<Option<SetupToken>>>` 里（进程内，重启即失效——这是**刻意的**，重启会重新打印新链接）。
  - 启动时（`crates/server/src/main.rs`，在 `routes::router(...)` 之前）：若 `mode == Team` 且库里无 active admin，则生成令牌并 `tracing::info!` 打印
    ```
    首次启动：请在 30 分钟内打开 http://<host>:<port>/login?setup=<token> 创建管理员
    ```
    **host 用实际监听地址**（`main_listener.local_addr()`，`main.rs:118`），端口用 `actual_main_port`。
  - `GET /api/local-auth/setup?token=...` → 返回 `{ valid: bool }`（供前端决定是否渲染向导）；**`valid=false` 时不消费令牌**。
  - `POST /api/local-auth/setup` → 建 admin + 直接建会话下发 Cookie。
- [ ] **D4.4 跑通 + 手工冒烟**：
  ```
  rm -f dev_assets/db.v2.sqlite   # 注意：会清空开发数据，先备份
  VK_MODE=team BACKEND_PORT=8080 cargo run --bin server
  # 控制台应打印初始化链接
  ```
  **注意：迁移会插入 `local` 这条 admin 用户行（A2.3），所以「库里无 active admin」的判断必须排除它，或者把 `local` 的 role 在 team 模式首启时视作「未初始化」。**
  → **决定：判断条件改为「不存在 `id != DEFAULT_USER_ID` 且 `role='admin'` 且 `status='active'` 的用户」**。并加测试 `只有固定本机用户时仍然算未初始化`。
- [ ] **D4.5 提交** `git add crates/server/src/routes/local_auth/setup.rs crates/services/src/services/local_auth/runtime.rs crates/server/src/main.rs shared/types.ts` / `git commit -m "团队版：首启管理员初始化向导"`

**任务 D 验收：**
```
cargo test -p server admin
cargo test -p server setup
cargo test -p db local_invite
pnpm run generate-types:check
```

---

## 任务 E：第三方登录（飞书 / Lark / Google）

**目标：** OAuth2 统一抽象 + 三份预置配置 + 回调 + 账号绑定；用本地 mock IdP 覆盖全部失败分支。
**依赖：** A、D。

> **⚠️ 本任务里所有第三方端点 URL、字段名、响应包裹格式都是「待核实」。** 本机无法访问真实提供方；执行者必须以官方文档或真实抓包为准，并把核实结果写回本节。计划里给出的值只是初值。

### E1 · OAuth 抽象与预置配置

**Create:** `crates/services/src/services/local_auth/oauth.rs`

- [ ] **E1.1 写测试**：
  - `三个提供方都有预置配置`：`provider_config("feishu" / "lark" / "google")` 都是 `Some`
  - `未知提供方返回_None`：`provider_config("github")` → `None`；`provider_config("../../etc")` → `None`（防路径注入进 URL）
  - `未配置凭据的提供方不出现在可用列表`
  - `授权 URL 包含 state 与 redirect_uri 且被正确编码`：对 `state` 里塞 `a&b=c`，断言出现 `state=a%26b%3Dc`
  - `redirect_uri 由 public_base_url 拼出`：`public_base_url = "https://k.example"` → `https://k.example/api/local-auth/oauth/feishu/callback`
  - **`public_base_url 未配置时 start 接口报 400 而不是拼出畸形 URL`**
  - **`public_base_url 末尾斜杠被规整`**：`"https://k.example/"` → 同上（不出现 `//api`）
  - `state 存储是一次性的`：`OAuthStateStore::consume(state)` 第二次返回 `None`
  - `state 10 分钟后过期`（注入时钟）
  - `state 存储有容量上限`：插入 10_001 条后最老的被淘汰（防内存耗尽）
  - **`state 与 provider 绑定`**：用 feishu 的 state 去 google 的回调 → `None`
- [ ] **E1.2 跑确认失败** `cargo test -p services oauth`。
- [ ] **E1.3 实现**：
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum TokenResponseStyle { Standard, FeishuEnvelope }

  #[derive(Debug, Clone)]
  pub struct OAuthProviderConfig {
      pub id: &'static str,
      pub authorize_url: &'static str,
      pub token_url: &'static str,
      pub userinfo_url: &'static str,
      pub scopes: &'static str,
      pub subject_field: &'static str,
      pub email_field: &'static str,
      pub name_field: &'static str,
      pub token_style: TokenResponseStyle,
      pub userinfo_envelope: bool,   // 响应是否包在 {"code":0,"data":{...}} 里
  }
  ```
  **初值（全部待核实）：**
  | id | authorize_url | token_url | userinfo_url | subject/email/name |
  |---|---|---|---|---|
  | `feishu` | `https://open.feishu.cn/open-apis/authen/v1/authorize` | `https://open.feishu.cn/open-apis/authen/v2/oauth/token` | `https://open.feishu.cn/open-apis/authen/v1/user_info` | `open_id` / `email` / `name` |
  | `lark` | `https://open.larksuite.com/open-apis/authen/v1/authorize` | `https://open.larksuite.com/open-apis/authen/v2/oauth/token` | `https://open.larksuite.com/open-apis/authen/v1/user_info` | 同上 |
  | `google` | `https://accounts.google.com/o/oauth2/v2/auth` | `https://oauth2.googleapis.com/token` | `https://openidconnect.googleapis.com/v1/userinfo` | `sub` / `email` / `name` |

  为了能用 mock IdP 测试，`OAuthProviderConfig` 的 URL 字段要能在测试里被覆盖：提供 `pub fn with_base_url(self, base: &str) -> OwnedProviderConfig`，把三个 URL 的 scheme+host 换成 mock 服务的地址。
- [ ] **E1.4 跑通 + 提交** `git add crates/services/src/services/local_auth/oauth.rs` / `git commit -m "团队版：OAuth2 提供方抽象与三份预置配置"`

### E2 · 本地 mock IdP

**Create:** `crates/services/src/services/local_auth/mock_idp.rs`（`#[cfg(test)]`）

- [ ] **E2.1 实现**：用 `axum` + `tokio::net::TcpListener::bind("127.0.0.1:0")` 起一个测试服务器，返回 `(base_url, JoinHandle, Arc<MockState>)`。`MockState` 可配置：
  - `token_status: StatusCode`、`token_body: serde_json::Value`
  - `userinfo_status`、`userinfo_body`
  - `expect_code: String`（拿到不匹配的 code 就返回 400）
  - 记录收到的请求，供断言。
  **不引入 `wiremock`**（不在 `Cargo.lock` 里，services 已依赖 axum？——**待核实：`crates/services/Cargo.toml` 当前没有 axum 依赖**。若没有，改用 `tokio` + 手写最小 HTTP 响应，或把 mock IdP 与 OAuth 回调测试整体放到 `crates/server` 的 `#[cfg(test)]` 里，因为 `crates/server` 已依赖 axum。**本计划选后者：mock IdP 放 `crates/server/src/routes/local_auth/oauth_routes.rs` 的测试模块。**）
- [ ] **E2.2 提交**（与 E3 合并提交亦可）

### E3 · 回调与绑定

**Create:** `crates/server/src/routes/local_auth/oauth_routes.rs`

- [ ] **E3.1 写攻击样例测试**（全部用 mock IdP）：
  | 用例 | 预期 |
  |---|---|
  | `完整流程：已绑定身份 → 建会话登录` | 302 到 `/`，带 `Set-Cookie: vk_session` |
  | **`state 不存在 → 400`** | `BadRequest("登录已过期，请重试")`，**不建会话** |
  | **`state 已用过 → 400`** | 同上（重放攻击） |
  | **`state 过期 → 400`** | 同上 |
  | **`state 属于另一个 provider → 400`** | 同上 |
  | `token 端点返回 500 → 502` | `BadGateway` |
  | `token 端点返回 200 但无 access_token → 502` | |
  | `token 端点返回飞书信封 code!=0 → 502` | 覆盖 `TokenResponseStyle::FeishuEnvelope` |
  | `userinfo 返回 401 → 502` | |
  | **`userinfo 缺 subject 字段 → 502 且不建用户`** | 断言 `local_users` 行数未变 |
  | `userinfo 的 subject 是空串 → 502` | |
  | `userinfo 缺 email → 仍可登录（email 为 NULL）` | |
  | **`(provider, subject) 已绑定到用户 A，再次回调不会改绑到别人`** | |
  | **`subject 冲突：两个不同用户绑同一 (provider, subject) → 409`** | 绑定接口返回 `Conflict` |
  | **`email 撞号不自动合并`** | 库里已有 `alice@x.com`，OAuth 返回同一 email 且未绑定 → **不登录**，返回 403 `"该邮箱已被占用，请登录后在设置里绑定"`；断言没有新建用户、没有新建 identity、没有建会话 |
  | `allow_oauth_signup=false 且未绑定 → 403 "请联系管理员开通"` | |
  | `allow_oauth_signup=true 且未绑定且 email 不撞号 → 建 member 用户并登录` | |
  | `新建用户的 username 由 subject 派生且规范化` | 冲突时加数字后缀；断言不会撞唯一约束 |
  | **`回调不写 access_token 到库`** | 断言 `local_user_identities` 的列里没有 token 字段（schema 层面已保证），并且日志里不出现 token（代码审查项） |
  | `被停用用户的身份回调 → 403` | 不建会话 |
  | `personal 模式下 oauth 路由返回 404` | |
  | `provider 路径参数非法（如 "../x"）→ 404` | |
  | `绑定接口要求已登录` | 未登录 → 401 |
  | **`绑定接口不能把已属于他人的 identity 抢过来`** | 409 |
- [ ] **E3.2 跑确认失败**。
- [ ] **E3.3 实现**：
  - `GET /start`：生成 state（32 字节 base64url）存入 `OAuthStateStore`（带 provider 与过期时间）→ 302 到 authorize_url。
  - `GET /callback`：见上表流程。成功后 302 到 `/`（同源），**不要把令牌放进 URL**。
  - `POST /bind`：已登录用户发起绑定，同样走 start/callback，但 state 里额外记 `bind_for_user_id`。
  - HTTP 客户端用 `reqwest`（`crates/server/Cargo.toml:52` 已依赖），**必须设超时**（`.timeout(Duration::from_secs(10))`），否则挂死的 IdP 会占满连接。
- [ ] **E3.4 跑通** `cargo test -p server oauth_routes`（预期 25+ 条）。
- [ ] **E3.5 提交** `git add crates/server/src/routes/local_auth/oauth_routes.rs crates/db/src/models/local_auth.rs crates/db/.sqlx shared/types.ts` / `git commit -m "团队版：飞书/Lark/Google 登录与账号绑定"`

**任务 E 验收：**
```
cargo test -p server oauth
cargo test -p services oauth
cargo test --workspace
```
**外加人工项（写进 §12）：** 上线前必须用真实凭据各走通一次，并把实际的端点 URL / 字段名回填到 `oauth.rs`。

---

## 任务 F：数据层（WAL + 连接池）

**目标：** SQLite 切 WAL 并验证变更钩子在 WAL 下行为一致、备份不出错、可回退；连接池显式上限。
**依赖：** 无（可与 A 并行）。

### F1 · WAL 开关

**Modify:** `crates/db/src/lib.rs:77-86`（`main_db_options`）、`:102-108`（`new_at_path`）、`:136-165`（`create_pool`）、`crates/local-deployment/src/lib.rs:139-147`

- [ ] **F1.1 写失败测试**（`crates/db/src/lib.rs` 的 `mod tests`）：
  - `默认走_WAL`：`TestDb::new()` 后 `PRAGMA journal_mode` 返回 `"wal"`
  - `可回退到_delete`：`DBService::new_at_path_with_journal(path, SqliteJournalMode::Delete)` 后返回 `"delete"`
  - `WAL 下 synchronous 是 NORMAL`：`PRAGMA synchronous` 返回 `1`
  - `busy_timeout 仍然生效`（既有测试 `连接自带写锁等待时间`，:174 不改）
  - `连接池上限是 16`：`pool.options().get_max_connections() == 16`（**待核实 sqlx 0.8 是否暴露 `options()`**；若没有，改成断言常量 `MAX_CONNECTIONS == 16` 并在构造处使用）
- [ ] **F1.2 跑确认失败** `cargo test -p db 默认走_WAL` → 预期 `assertion failed: left == right, left: "delete", right: "wal"`。
- [ ] **F1.3 实现**：
  ```rust
  pub const MAX_CONNECTIONS: u32 = 16;

  fn journal_mode_from_env() -> SqliteJournalMode {
      match std::env::var("VK_SQLITE_WAL").as_deref() {
          Ok("0") | Ok("false") | Ok("FALSE") => SqliteJournalMode::Delete,
          _ => SqliteJournalMode::Wal,
      }
  }

  fn main_db_options() -> Result<SqliteConnectOptions, Error> {
      Ok(SqliteConnectOptions::from_str(&database_url)?
          .create_if_missing(true)
          .journal_mode(journal_mode_from_env())
          .synchronous(SqliteSynchronous::Normal)
          .busy_timeout(BUSY_TIMEOUT))
  }
  ```
  三条路径（`new`、`new_at_path`、`create_pool`、`new_migration_pool`）都要走同一个 options 构造，与 :77-79 的既有注释意图一致。所有 `SqlitePool::connect_with` 换成 `SqlitePoolOptions::new().max_connections(MAX_CONNECTIONS).connect_with(...)`；`new_migration_pool`（:114-120）保持 64 不变（它是一次性迁移池）。
  **注意：`VK_SQLITE_WAL` 在这里直接读 `std::env::var` 与 A1「不要直接读环境变量」的建议冲突。** 这里可以接受，因为它在进程启动早期只读一次且测试用 `new_at_path_with_journal` 显式传参绕开；但要把 `journal_mode_from_env` 也做成 `fn journal_mode_from(value: Option<&str>) -> SqliteJournalMode` 的薄封装，并对 `None`/`"0"`/`"false"`/`"1"`/`"garbage"` 各写一条测试。
- [ ] **F1.4 跑通** `cargo test -p db`。
- [ ] **F1.5 提交** `git add crates/db/src/lib.rs` / `git commit -m "数据层：SQLite 切 WAL，连接池设上限，保留回退开关"`

### F2 · WAL 下的变更钩子验证

**Create/Modify:** `crates/services/src/services/events.rs` 的 `#[cfg(test)] mod tests`（当前该文件**没有**测试模块，需要新建）

- [ ] **F2.1 写测试**：这是本任务的核心风险项（设计文档 §9 明确要求验证）。
  - `WAL 下 update_hook 仍然触发`：用 `TestDb`（已是 WAL）+ `DBService::new_with_after_connect` 不可用（它读 `asset_dir()`）。
    → **需要给 `DBService` 加一个 `new_at_path_with_after_connect(path, hook)`**，否则无法在测试里装钩子。把它加进 `crates/db/src/lib.rs` 并在 `test_support` 里暴露 `TestDb::new_with_hook(hook)`。
  - 用例：插入一条 `issues` 行后，`MsgStore` 里应出现一个 `add` patch（读取方式参考 `crates/services/src/services/events/patches.rs` 的 `issue_patch::add`，断言 patch 的 `op`/`path`）。
  - `WAL 下 preupdate_hook 仍能取到第 0 列`：删除一条 `issues` 行后出现 `remove` patch。
  - `WAL 下并发读不被写阻塞`：起一个写事务（不提交）+ 另一个连接读，读应立即返回而不是等 10 秒 busy_timeout。用 `tokio::time::timeout(Duration::from_secs(2), ...)` 断言。
  - `Delete 模式下同样的两条钩子测试也通过`（参数化，证明切换不改变行为）。
- [ ] **F2.2 跑确认失败**（先失败于 `new_at_path_with_after_connect` 不存在）。
- [ ] **F2.3 实现并跑通** `cargo test -p services events`。
- [ ] **F2.4 提交** `git add crates/db/src/lib.rs crates/db/src/test_support.rs crates/services/src/services/events.rs` / `git commit -m "数据层：验证变更钩子在 WAL 下行为一致"`

### F3 · WAL 对备份的影响

- [ ] **F3.1 写脚本级测试**（放在 `deploy/backup.sh` 之后做，见任务 I4）。本步只在 `crates/db/src/lib.rs` 加一条测试：
  - `WAL 模式下直接 cp 主库文件会丢数据`：写一条数据（不 checkpoint）→ 只复制 `.sqlite`（不复制 `-wal`）→ 打开副本 → 断言数据**不在**。这条测试把「必须用 `sqlite3 .backup`」的理由钉在代码里。
    **若发现 sqlx 在连接关闭时自动 checkpoint 导致该测试不稳定**，改成断言 `PRAGMA journal_mode` 为 wal 时 `<db>-wal` 文件存在，并在注释里写明理由。
- [ ] **F3.2 提交** `git commit -m "数据层：用测试固化 WAL 下必须用 .backup 备份"`

**任务 F 验收：**
```
cargo test -p db
cargo test -p services events
cargo test --workspace
```

---

## 任务 G：前端认证接入

**目标：** 运行时模式（替换构建期 `VITE_VK_DATA_SOURCE`）、CSRF 头、登录页、会话过期处理、用户菜单、成员管理界面。
**依赖：** A、C、D（E 可选）。

> **测试约束（§2.8）：所有新测试必须是 `packages/web-core/src/**/*.test.ts`，node 环境，不能渲染 React。** 所以每个任务都先抽纯函数、测纯函数，React 组件只做装配。
> **i18n 约束：新增的每个 key 必须同时写进 7 个语言目录**（`packages/web-core/src/i18n/locales/{en,es,fr,ja,ko,zh-Hans,zh-Hant}/common.json`），且必须在源码里被引用，否则 `pnpm lint` 与 CI 的 `check-i18n.sh` 会失败。

### G1 · 运行时模式

**Create:** `packages/web-core/src/shared/lib/local/runtimeMode.ts` + `.test.ts`
**Modify:** `packages/local-web/src/app/entry/Bootstrap.tsx:81-90`

- [ ] **G1.1 写失败测试**（`runtimeMode.test.ts`，照抄 `localEndpoints.test.ts` 的风格：具名 vitest 导入 + 中文用例名 + `beforeEach` 重置单例）：
  - `默认未初始化时 getRuntimeMode 抛错`（强制调用方必须先 `configureRuntimeMode`，避免静默按 personal 跑）
  - `配置 personal 后 isPersonalMode 为真且 requiresLogin 为假`
  - `配置 team 后 requiresLogin 为真`
  - `applyBootstrap 同时设置数据源`：`applyBootstrap({mode:'team',...})` 之后 `getDataSourceMode() === 'local'`（**注意：team 模式仍然是本地数据源**，`dataSource` 的 `'remote'` 指的是云端 ElectricSQL，跟登录模式是两个正交概念——这一点在代码注释里必须写清楚，否则很容易改错）
  - `未知 mode 值抛错`：`applyBootstrap({mode:'enterprise'})` → throw
- [ ] **G1.2 跑确认失败** `pnpm --filter @vibe/web-core test runtimeMode` → 预期 `Cannot find module '@/shared/lib/local/runtimeMode'`。
- [ ] **G1.3 实现**：
  ```ts
  export type RuntimeMode = 'personal' | 'team';
  let runtimeMode: RuntimeMode | null = null;
  export function configureRuntimeMode(mode: RuntimeMode): void;
  export function getRuntimeMode(): RuntimeMode;          // 未配置时 throw
  export function isPersonalMode(): boolean;
  export function requiresLogin(): boolean;
  export function resetRuntimeModeForTests(): void;       // 仅测试用
  ```
- [ ] **G1.4 跑通** `pnpm --filter @vibe/web-core test runtimeMode`。
- [ ] **G1.5 提交** `git add packages/web-core/src/shared/lib/local/runtimeMode.ts packages/web-core/src/shared/lib/local/runtimeMode.test.ts` / `git commit -m "前端：运行时模式单例"`

### G2 · CSRF 令牌读取

**Create:** `packages/web-core/src/shared/lib/local/csrf.ts` + `.test.ts`
**Modify:** `packages/web-core/src/shared/lib/localApiTransport.ts:111-116`

- [ ] **G2.1 写失败测试**（`readCsrfTokenFrom(cookieString)` 做成接收字符串的纯函数，便于 node 环境测试；`readCsrfToken()` 再包一层读 `document.cookie`）：
  - `读到 vk_csrf`：`readCsrfTokenFrom('a=1; vk_csrf=xyz; b=2') === 'xyz'`
  - `没有时返回 null`
  - **`不前缀匹配`**：`'vk_csrf_x=bad'` → `null`
  - **`不后缀匹配`**：`'xvk_csrf=bad'` → `null`
  - `空值返回 null`：`'vk_csrf='` → `null`（空串当作没有）
  - `大小写敏感`：`'VK_CSRF=x'` → `null`
  - `重复时取第一个`：`'vk_csrf=a; vk_csrf=b'` → `'a'`
  - `值被 decodeURIComponent`：`'vk_csrf=a%2Bb'` → `'a+b'`（**若服务端不做 URL 编码就不要解码**——服务端令牌是 base64url，本来就无需编码。→ **决定：不解码**，测试改为 `'vk_csrf=a%2Bb'` → `'a%2Bb'`，并在注释写明服务端保证令牌字符集为 `[A-Za-z0-9_-]`）
  - `空字符串输入返回 null`
- [ ] **G2.2 跑确认失败**。
- [ ] **G2.3 实现** `csrf.ts`。
- [ ] **G2.4 接进请求收口**：`localApiTransport.ts:111-116` 改成
  ```ts
  export async function makeLocalApiRequest(
    pathOrUrl: string,
    init: LocalApiRequestOptions = {}
  ): Promise<Response> {
    const method = (init.method ?? 'GET').toUpperCase();
    const needsCsrf = method !== 'GET' && method !== 'HEAD' && method !== 'OPTIONS';
    const headers = new Headers(init.headers ?? {});
    if (needsCsrf) {
      const token = readCsrfToken();
      if (token) headers.set('X-VK-CSRF', token);
    }
    return transport.request(resolveScopedPath(pathOrUrl, init), {
      ...init,
      headers,
      credentials: init.credentials ?? 'same-origin',
    });
  }
  ```
  **注意：`init.headers` 可能是 `Headers`、`Record<string,string>` 或数组，`new Headers(...)` 三种都吃得下。** 现有调用方 `api.ts:128-138` 已经传 `Headers` 实例，不会冲突。
  写一条测试 `写方法会带上 CSRF 头`：用 `setLocalApiTransport` 注入一个假 transport 记录收到的 init，再断言。**这条测试需要能读到 cookie**——在 node 环境下 `document` 不存在，所以 `readCsrfToken()` 必须在 `typeof document === 'undefined'` 时返回 `null` 而不是抛错，并让测试通过依赖注入（`makeLocalApiRequest` 内部调用的是从 `csrf.ts` 导入的函数，用 `vi.mock('@/shared/lib/local/csrf')` 替换）。
- [ ] **G2.5 跑通** `pnpm --filter @vibe/web-core test csrf` 与 `... test localApiTransport`。
- [ ] **G2.6 提交** `git add packages/web-core/src/shared/lib/local/csrf.ts packages/web-core/src/shared/lib/local/csrf.test.ts packages/web-core/src/shared/lib/localApiTransport.ts packages/web-core/src/shared/lib/localApiTransport.test.ts` / `git commit -m "前端：写请求携带双提交 CSRF 令牌"`

### G3 · bootstrap / 登录 API 客户端

**Create:** `packages/web-core/src/shared/lib/local/bootstrapApi.ts` + `.test.ts`

- [ ] **G3.1 写失败测试**（用 `vi.stubGlobal('fetch', ...)`）：
  - `fetchBootstrap 解析 ApiResponse 包裹`：mock 返回 `{success:true,data:{mode:'team',...}}` → 返回 `data`
  - `fetchBootstrap 在 success=false 时抛错`
  - `fetchBootstrap 在 5xx 时抛错且错误信息含状态码`
  - `login 成功返回用户`
  - **`login 在 401 时抛出 InvalidCredentialsError 而不是通用错误`**（登录页需要区分展示）
  - **`login 在 429 时抛出 RateLimitedError`**
  - `logout 即使 401 也 resolve`（幂等）
  - `所有请求都走 makeLocalApiRequest`（断言 mock transport 被调用，而不是裸 fetch——保证 CSRF 头生效）
- [ ] **G3.2 跑确认失败 → 实现 → 跑通**。
- [ ] **G3.3 提交** `git add packages/web-core/src/shared/lib/local/bootstrapApi.ts packages/web-core/src/shared/lib/local/bootstrapApi.test.ts` / `git commit -m "前端：本地认证 API 客户端"`

### G4 · 入口改造与会话上下文

**Modify:** `packages/local-web/src/app/entry/Bootstrap.tsx:81-90`、`packages/local-web/src/app/entry/App.tsx:20-45`、`packages/web-core/src/shared/hooks/auth/useAuth.ts`
**Create:** `packages/web-core/src/shared/providers/auth/LocalSessionProvider.tsx`

- [ ] **G4.1 改 Bootstrap**：把 :81-90 的构建期判断整块替换为
  ```tsx
  // 运行时模式：由 /api/local-auth/bootstrap 决定，不再依赖构建期 VITE_VK_DATA_SOURCE。
  const bootstrap = await fetchBootstrap();
  applyBootstrap(bootstrap);
  ```
  但 Bootstrap.tsx 是顶层模块代码（不是组件），目前是同步的。**改法：把 `ReactDOM.createRoot(...).render(...)` 包进一个 `async function main()` 并在末尾 `void main();`**；在 `fetchBootstrap()` 失败时（后端没起来 / 网络错误）渲染一个最小的错误页而不是白屏——写进 `packages/ui/src/components/CrashScreen`（已存在，`Bootstrap.tsx:9` 已导入）。
  同时：**删掉 `VITE_VK_DATA_SOURCE` 的所有痕迹**（只有 :81-83 两处），并在 `packages/web-core/src/vite-env.d.ts` 里不要新增声明。
- [ ] **G4.2 改 useAuth**：`packages/web-core/src/shared/hooks/auth/useAuth.ts` 里那段
  ```ts
  if (isLocalMode()) {
    return { isSignedIn: true, isLoaded: true, userId: LOCAL_USER_ID };
  }
  ```
  改成 `if (isPersonalMode()) { ... }`（personal 才恒为已登录；team 走 context）。
  **这是 §2.7 列的 5 号坑：不改这里，团队版的登录 UI 会全部不可见。**
- [ ] **G4.3 写 LocalSessionProvider**：用 `useQuery(['local-auth','me'], fetchMe)`，提供 `{ user, isLoading, refetch, signOut }`，并把 `AuthContextValue`（`useAuth.ts:6-10`）填好。挂在 `App.tsx:26` 现有 `<LocalAuthProvider>` 的**外层或替换位置**——`LocalAuthProvider` 依赖 `useUserSystem().loginStatus`（云端），team 模式下应改用新 Provider。
  **决定：在 `App.tsx` 里按 `getRuntimeMode()` 二选一渲染**，避免两个 Provider 同时写 `AuthContext`。
- [ ] **G4.4 会话过期处理**：在 `packages/web-core/src/shared/lib/localApiTransport.ts` 之外单独加一个「401 拦截」——**不要在 transport 里做**（transport 是纯管道）。改在 `api.ts:248+` 的 `handleApiResponse` 里：遇到 401 且 `requiresLogin()` 为真时，触发一次全局事件 `window.dispatchEvent(new CustomEvent('vk:session-expired'))`；`LocalSessionProvider` 监听该事件并跳转 `/login?redirect=<当前路径>`。
  纯函数测试：`shouldTreatAsSessionExpired(status, mode)`：
  - `(401, 'team') === true`
  - `(401, 'personal') === false`
  - `(403, 'team') === false`（CSRF 失败不是会话过期，见 C2.4）
  - `(500, 'team') === false`
- [ ] **G4.5 跑通** `pnpm --filter @vibe/web-core test && pnpm run local-web:check && pnpm run web-core:check`。
- [ ] **G4.6 提交** `git add packages/local-web/src/app/entry/Bootstrap.tsx packages/local-web/src/app/entry/App.tsx packages/web-core/src/shared/hooks/auth/useAuth.ts packages/web-core/src/shared/providers/auth/LocalSessionProvider.tsx packages/web-core/src/shared/lib/api.ts` / `git commit -m "前端：运行时 bootstrap 驱动模式与会话上下文"`

### G5 · 登录页

**Create:** `packages/ui/src/components/LoginPanel.tsx`（视图，无状态）、`packages/web-core/src/features/local-auth/ui/LoginPageContainer.tsx`（容器）、`packages/local-web/src/routes/login.tsx`（路由）

- [ ] **G5.1 先抽纯逻辑并测试**：`packages/web-core/src/features/local-auth/model/loginForm.ts` + `.test.ts`
  - `validateLoginForm({username:'',password:'x'})` → `{ username: 'errors.usernameRequired' }`
  - `validateLoginForm({username:'a',password:''})` → `{ password: 'errors.passwordRequired' }`
  - `合法输入返回空错误对象`
  - **`重定向目标只允许同源相对路径`**：`safeRedirect('/projects/1')` → `'/projects/1'`；`safeRedirect('//evil.example')` → `'/'`；`safeRedirect('http://evil.example')` → `'/'`；`safeRedirect('/\\evil')` → `'/'`；`safeRedirect(null)` → `'/'`（开放重定向防护，必须有测试）
- [ ] **G5.2 跑确认失败 → 实现 → 跑通**。
- [ ] **G5.3 写视图** `LoginPanel.tsx`：props 全部受控（`username, password, error, isSubmitting, providers, onUsernameChange, onPasswordChange, onSubmit, onProviderClick`）。样式用设计令牌（依据 `packages/local-web/tailwind.new.config.js:103-125`）：
  - 容器 `bg-primary text-normal`
  - 卡片 `bg-secondary rounded border p-double`
  - 输入框 `px-base bg-panel rounded border text-base text-normal placeholder:text-low focus:outline-none focus:ring-1 focus:ring-brand`
  - 主按钮 `bg-brand text-on-brand hover:bg-brand-hover h-cta`
  - 错误 `text-error`
  **暗色适配自动生效**（`.dark` 下 token 重定义，`packages/web-core/src/app/styles/new/index.css:90-162`），但必须人工在两种主题下看一遍（验收项）。
  **不要用 `lucide-react`**（`packages/local-web/.eslintrc.cjs` 对 `ui-new` 禁用）；`packages/ui` 用 `@phosphor-icons/react`。
- [ ] **G5.4 写容器 + 路由**：`packages/local-web/src/routes/login.tsx`
  ```tsx
  export const Route = createFileRoute('/login')({ component: LoginPageContainer });
  ```
  **文件名必须小写 `login.tsx`**（`routes/**` 被 eslint 的 `check-file/filename-naming-convention` 豁免，见 `.eslintrc.cjs` 的 `files: ['src/routes/**/*.{ts,tsx}', 'src/routeTree.gen.ts']` override）。加完后 `routeTree.gen.ts` 由 `@tanstack/router-plugin` 自动重生成——**跑一次 `pnpm --filter @vibe/local-web run build` 或 `dev` 让它生成，然后把 `routeTree.gen.ts` 一起提交**。
- [ ] **G5.5 加 i18n key**：新增 namespace 用 `common`。key 列表（示例）：`localAuth.signInTitle`、`localAuth.username`、`localAuth.password`、`localAuth.signIn`、`localAuth.signingIn`、`localAuth.invalidCredentials`、`localAuth.rateLimited`、`localAuth.orContinueWith`、`localAuth.providerFeishu`、`localAuth.providerLark`、`localAuth.providerGoogle`。
  **7 个语言文件都要加**（非 en 可先填英文原文，`check_key_consistency` 只比 key 不比值）：
  ```
  packages/web-core/src/i18n/locales/{en,es,fr,ja,ko,zh-Hans,zh-Hant}/common.json
  ```
  加完跑 `node scripts/check-unused-i18n-keys.mjs`，预期 `✅`（每个 key 都已在源码里引用）。
- [ ] **G5.6 跑通** `pnpm run check && pnpm run lint`。
- [ ] **G5.7 提交** `git add packages/ui/src/components/LoginPanel.tsx packages/web-core/src/features/local-auth/ packages/local-web/src/routes/login.tsx packages/local-web/src/routeTree.gen.ts packages/web-core/src/i18n/locales/*/common.json` / `git commit -m "前端：本地登录页"`

### G6 · 用户菜单适配

**Modify:** `packages/web-core/src/shared/components/ui-new/containers/AppBarUserPopoverContainer.tsx`、`packages/ui/src/components/AppBarUserPopover.tsx`、`packages/web-core/src/shared/actions/index.ts:443-474`

- [ ] **G6.1 写纯函数测试**：`packages/web-core/src/features/local-auth/model/userMenu.ts` + `.test.ts`
  - `buildUserMenuItems({mode:'personal'})` → 不含「登录」「退出」「成员管理」
  - `buildUserMenuItems({mode:'team', role:'member'})` → 含「退出」，不含「成员管理」
  - `buildUserMenuItems({mode:'team', role:'admin'})` → 含「成员管理」
  - `buildUserMenuItems({mode:'team', user:null})` → 只含「登录」
  - `头像首字母取 display_name 首个字符`：`initials('张三') === '张'`；`initials('alice') === 'A'`；`initials('') === '?'`；`initials('  ')` → `'?'`（全空白）
- [ ] **G6.2 跑确认失败 → 实现 → 跑通**。
- [ ] **G6.3 接进容器**：`AppBarUserPopoverContainer.tsx` 里按 `getRuntimeMode()` 决定用云端 `loginStatus` 还是本地 `LocalSessionProvider` 的 user。
  **`Actions.SignIn` / `Actions.SignOut`（`packages/web-core/src/shared/actions/index.ts:443,457`）当前 `isVisible` 依赖 `ctx.isSignedIn`，而 personal 模式下 `useAuth()` 恒为 true，所以「退出」按钮今天在个人版是可见的**（§2.7 的 9f 已核实）。本步顺手修掉：`isVisible: (ctx) => !isPersonalMode() && ctx.isSignedIn`。
  **注意 `packages/web-core/src/shared/components/ui-new/containers/**` 受 eslint 「容器组件不允许可选 props」约束（`.eslintrc.cjs` 的 `src/components/ui-new/containers/**`）——但那条规则只对 `packages/local-web` 生效，web-core 不过 eslint。仍建议遵守。**
- [ ] **G6.4 提交** `git add packages/web-core/src/features/local-auth/model/userMenu.ts packages/web-core/src/features/local-auth/model/userMenu.test.ts packages/web-core/src/shared/components/ui-new/containers/AppBarUserPopoverContainer.tsx packages/ui/src/components/AppBarUserPopover.tsx packages/web-core/src/shared/actions/index.ts` / `git commit -m "前端：用户菜单区分个人版与团队版"`

### G7 · 成员管理界面

**Create:** `packages/ui/src/components/MembersPanel.tsx`、`packages/web-core/src/features/local-auth/ui/MembersPageContainer.tsx`、`packages/local-web/src/routes/_app.members.tsx`

- [ ] **G7.1 写纯函数测试**：`packages/web-core/src/features/local-auth/model/members.ts` + `.test.ts`
  - `sortMembers` 按 role（admin 在前）再按 username
  - `canDisable(actor, target)`：自己 → false；最后一个 active admin → false；其它 → true
  - `canChangeRole(actor, target)`：自己 → false；actor 非 admin → false
  - `validateNewMember`：用户名字符集、密码长度、邮箱格式（简单 `/^[^@\s]+@[^@\s]+\.[^@\s]+$/`）
  - **`前端的禁用判断只是 UX，后端仍会拦`**：在测试注释里写明，并要求 D2 的后端测试覆盖同样规则（避免只在前端拦）
- [ ] **G7.2 跑确认失败 → 实现 → 跑通**。
- [ ] **G7.3 写视图与容器 + 路由**，路由文件 `_app.members.tsx`（跟随 `_app` 布局），重生成 `routeTree.gen.ts`。i18n key 加进 7 个 `common.json`。
- [ ] **G7.4 个人版隐藏入口**：`isPersonalMode()` 时路由直接 redirect 到 `/`，并且用户菜单不显示入口（G6 已覆盖）。
- [ ] **G7.5 跑通** `pnpm run check && pnpm run lint`。
- [ ] **G7.6 提交** `git add packages/ui/src/components/MembersPanel.tsx packages/web-core/src/features/local-auth/ packages/local-web/src/routes/_app.members.tsx packages/local-web/src/routeTree.gen.ts packages/web-core/src/i18n/locales/*/common.json` / `git commit -m "前端：成员管理界面"`

**任务 G 验收：**
```
pnpm --filter @vibe/web-core test      # 预期新增 40+ 条用例全绿
pnpm run check                          # legacy-path-guard + 4 个 tsc + backend:check + web-core:test
pnpm run lint                           # eslint + clippy + 未使用 i18n key
GITHUB_BASE_REF=main ./scripts/check-i18n.sh   # 预期「✅ Translation keys are consistent across locales.」
```
人工验收：个人版启动后界面与改造前一致（无登录页、无成员入口）；`VK_MODE=team` 启动后未登录访问任意页面被导到 `/login`。

---

## 任务 H：界面优化（设计文档 7.1～7.5）

**目标：** 逐屏改造，不推倒重来。
**依赖：** G（模式开关）；H1 依赖后端。

> **反复强调：`KanbanContainer.tsx` 1153 行、`KanbanIssuePanelContainer.tsx` 1113 行，禁止重写。** 每一步只做「抽出一段 + 就地替换」，抽出后行为必须等价。

### H1 · 新增 `test` 流程阶段（后端）

**Modify:** `crates/db/src/models/local_project_status.rs:26-44,295-310`、`crates/db/src/models/local_project.rs:14-20`
**Create:** `crates/db/migrations/20260917010000_add_test_stage_status.sql`

- [ ] **H1.1 写失败测试**：
  - `StageType 有 Test 且字符串是 test`
  - `新建项目自带六个状态列且顺序是 backlog/todo/dev/review/test/done`（**同时改掉既有断言 `local_project_status.rs:304-305`**：`statuses[4].stage_type` 从 `"done"` 改成 `"test"`，新增 `statuses[5].stage_type == "done"`）
  - `迁移给已有项目补上测试中列且 sort_order 在 review 与 done 之间`
  - **`已有「测试中」列的项目不会被重复插入`**（迁移用 `WHERE NOT EXISTS`）
  - `迁移可重复执行`（复用 `test_support.rs` 的既有套路）
- [ ] **H1.2 跑确认失败** `cargo test -p db stage` → 预期 `no variant named Test`。
- [ ] **H1.3 实现**：
  - `StageType` 加 `Test`，`as_str` 加 `StageType::Test => "test"`。
  - `DEFAULT_STATUSES` 类型 `[(&str,&str,&str); 5]` → `; 6]`，在 `("待评审",...)` 与 `("已完成",...)` 之间插入 `("测试中", "#06b6d4", "test")`。
  - 迁移：
    ```sql
    -- 给已有项目补一条「测试中」状态列，插在 review 与 done 之间。
    -- sort_order 用 review 的 sort_order + 1，并把原本 >= 该值的列整体后移，
    -- 避免与 done 撞号（project_statuses 没有 sort_order 唯一约束，撞号只会让顺序不稳定）。
    UPDATE project_statuses
    SET sort_order = sort_order + 1
    WHERE project_id IN (SELECT id FROM local_projects)
      AND sort_order > (
        SELECT COALESCE(MAX(s.sort_order), -1) FROM project_statuses s
        WHERE s.project_id = project_statuses.project_id AND s.stage_type = 'review'
      );

    INSERT INTO local_projects_test_stage_tmp ... ;  -- 见下
    ```
    **SQLite 不支持在 INSERT ... SELECT 里生成 UUID。** 解决办法：用 `randomblob(16)` 生成主键：
    ```sql
    INSERT INTO project_statuses (id, project_id, name, color, sort_order, hidden, stage_type)
    SELECT randomblob(16), p.id, '测试中', '#06b6d4',
           COALESCE((SELECT s.sort_order FROM project_statuses s
                     WHERE s.project_id = p.id AND s.stage_type = 'review'), 0) + 1,
           0, 'test'
    FROM local_projects p
    WHERE NOT EXISTS (
        SELECT 1 FROM project_statuses s2
        WHERE s2.project_id = p.id AND s2.stage_type = 'test'
    );
    ```
    **待核实：`randomblob(16)` 生成的 16 字节能否被 sqlx 解码成 `Uuid`。** 执行者先写一条测试插入后 `LocalProjectStatus` 查询能否成功解码；若不行，改成在应用层做一次性回填（启动时检查并补列），并把这条记进 §12。
- [ ] **H1.4 跑通** `cargo test -p db && cargo test --workspace`；`pnpm run prepare-db`。
- [ ] **H1.5 提交** `git add crates/db/migrations/20260917010000_add_test_stage_status.sql crates/db/src/models/local_project.rs crates/db/src/models/local_project_status.rs crates/db/.sqlx` / `git commit -m "界面：新增测试中流程阶段"`

### H2 · 前端 `stage_type` 类型与排序

**Create:** `packages/web-core/src/features/kanban/model/stageType.ts` + `.test.ts`

背景：`shared/remote-types.ts:19` 的 `ProjectStatus` **没有** `stage_type` 字段（由 remote crate 生成，不能改），但 `/api/local/project_statuses` 的响应里有（`crates/db/src/models/local_project_status.rs:20`）。

- [ ] **H2.1 写失败测试**：
  - `readStageType 从带额外字段的对象里取出 stage_type`
  - `未知值归一成 'todo'`：`readStageType({stage_type:'qa'})` → `'todo'`
  - `字段缺失归一成 'todo'`
  - `STAGE_ORDER 的顺序是 backlog<todo<dev<review<test<done`
  - `stageLabelKey 返回 i18n key 而不是硬编码中文`
  - **`团队版 remote 数据源下 stage_type 一定缺失，函数不能抛错`**
- [ ] **H2.2 跑确认失败 → 实现 → 跑通**：
  ```ts
  export type StageType = 'backlog' | 'todo' | 'dev' | 'review' | 'test' | 'done';
  export const STAGE_ORDER: Record<StageType, number>;
  export function readStageType(status: { stage_type?: unknown }): StageType;
  export function stageLabelKey(stage: StageType): string;
  ```
  在 `.ts` 里定义一个本地扩展类型 `type LocalProjectStatus = ProjectStatus & { stage_type?: string }`，**不要改 `shared/remote-types.ts`**。
- [ ] **H2.3 提交** `git add packages/web-core/src/features/kanban/model/stageType.ts packages/web-core/src/features/kanban/model/stageType.test.ts` / `git commit -m "界面：前端流程阶段类型"`

### H3 · 抽出筛选纯函数（为后续所有筛选改动铺路）

**Create:** `packages/web-core/src/features/kanban/model/filterIssues.ts` + `.test.ts`
**Modify:** `packages/web-core/src/features/kanban/model/hooks/useKanbanFilters.ts:74-177`

- [ ] **H3.1 写失败测试**：把 `useKanbanFilters` 的筛选逻辑（:74-165）整块搬成纯函数 `filterIssues(params): Issue[]`，参数与 `UseKanbanFiltersParams`（:14-25）一致但用普通对象（`assigneesByIssue` / `tagsByIssue` 作为入参，不在函数内部算）。测试覆盖：
  - `不显示子需求时过滤掉 parent_issue_id 非空的`
  - `关键字匹配标题 / simple_id / issue_number`
  - `关键字大小写不敏感且首尾空白被裁`
  - `优先级筛选是 OR`
  - `负责人筛选支持 unassigned 与 __self__`
  - `currentUserId 为 null 时 __self__ 匹配不到任何人`（现状行为，:115-117）
  - `标签筛选是 OR`
  - `hideBlocked 过滤掉被未完成需求阻塞的`
  - `空筛选返回原数组内容`
  - **`筛选函数不修改入参数组`**（断言 `issues` 引用与长度不变）
- [ ] **H3.2 跑确认失败** `pnpm --filter @vibe/web-core test filterIssues`。
- [ ] **H3.3 实现**：`filterIssues.ts` 里放纯函数；`useKanbanFilters.ts` 改成只算 `assigneesByIssue` / `tagsByIssue` 两个 `useMemo`（:51-71 保留），然后 `useMemo(() => filterIssues({...}), [...])`。**外部签名 `useKanbanFilters(params): { filteredIssues }` 完全不变**，`KanbanContainer.tsx:288-299` 不用改。
- [ ] **H3.4 跑通** `pnpm --filter @vibe/web-core test && pnpm run web-core:check`。
- [ ] **H3.5 提交** `git add packages/web-core/src/features/kanban/model/filterIssues.ts packages/web-core/src/features/kanban/model/filterIssues.test.ts packages/web-core/src/features/kanban/model/hooks/useKanbanFilters.ts` / `git commit -m "界面：抽出看板筛选纯函数并补齐单测"`

### H4 · 拆分 `KanbanContainer`（不改行为）

**Create:** `packages/web-core/src/features/kanban/ui/KanbanColumn.tsx`、`packages/web-core/src/features/kanban/ui/KanbanBoardView.tsx`
**Modify:** `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx:983-1126`

拆法（依据 §2 的块图）：
- `KanbanColumn.tsx` ← 原 :989-1125（单个 `<KanbanBoard>`：header + cards + 每张卡片 + 内嵌工作区卡片）。props 为纯数据 + 回调，无 hook。
- `KanbanBoardView.tsx` ← 原 :983-1126（`<KanbanProvider onDragEnd>` + `visibleStatuses.map`），内部渲染 `KanbanColumn`。
- `KanbanContainer.tsx` 保留 :126-894 的全部 hook 与 :896-981 的头部区，把 :983-1126 替换成 `<KanbanBoardView ...props />`。

- [ ] **H4.1 先写「行为等价」的护栏**：由于不能渲染 React，改为**类型护栏 + 快照护栏**：
  - 在 `packages/web-core/src/features/kanban/model/boardModel.ts` 里把「从 statuses + items + issues 计算出每列要渲染什么」抽成纯函数 `buildBoardColumns(...) : BoardColumn[]`，并为它写测试（列顺序、每列的卡片顺序、隐藏列不出现、计数正确）。
  - `KanbanColumn` 只消费 `BoardColumn`。
- [ ] **H4.2 跑确认失败 → 实现 `buildBoardColumns` → 跑通**。
- [ ] **H4.3 机械拆分**：把 JSX 原样搬过去，只把闭包变量改成 props。**搬完后 `git diff --stat` 里 `KanbanContainer.tsx` 应只有删除 + 一行新增。**
- [ ] **H4.4 验收** `pnpm run web-core:check && pnpm run local-web:check && pnpm run local-web:lint`。人工在浏览器里对照改造前后：列数、卡片顺序、拖拽、多选、列表视图切换都一致。
- [ ] **H4.5 提交** `git add packages/web-core/src/features/kanban/` / `git commit -m "界面：拆分 KanbanContainer 的看板渲染区（行为不变）"`

### H5 · 筛选写进 URL（7.5）

**Create:** `packages/web-core/src/features/kanban/model/kanbanUrlState.ts` + `.test.ts`
**Modify:** `packages/web-core/src/project-routes/project-search.ts`（当前 8 行，schema 为空对象）、`packages/local-web/src/routes/_app.projects.$projectId.tsx:6`

- [ ] **H5.1 写失败测试**：
  - `filtersToSearch 只输出非默认值`：默认筛选 → `{}`（URL 保持干净）
  - `searchToFilters 补齐默认值`
  - `往返一致`：`searchToFilters(filtersToSearch(f))` deep-equal `f`
  - **`非法 sortField 回落到 sort_order`**：`searchToFilters({sort:'DROP TABLE'})`
  - **`非法 priority 被丢弃而不是抛错`**：`{priority:['urgent','nope']}` → `['urgent']`
  - `数组参数用逗号分隔并被 URL 编码`
  - `超长 q 被截断到 200 字符`（防止畸形 URL）
  - `assigneeIds 里的 __self__ 原样保留`
- [ ] **H5.2 跑确认失败 → 实现 → 跑通**。
- [ ] **H5.3 接进路由**：`project-search.ts` 的 `projectSearchSchema` 从 `z.object({})` 改成实际 schema（`q`, `priority`, `assignee`, `tag`, `sort`, `dir`, `view`, `density` 全部 `.optional()`）。
  **`projectSearchValidator` 被 8 个路由文件用到**（§2 的路径修正清单），schema 放宽是向后兼容的（原来空对象会丢弃所有 search 参数）。改完跑 `pnpm run local-web:check` 确认 8 个路由文件都编译通过。
- [ ] **H5.4 容器里双向同步**：`KanbanContainer` 里把 `useUiPreferencesStore` 的筛选读写包一层：初次挂载时若 URL 有参数则以 URL 为准写入 store；store 变化时 `navigate({ search })` 更新 URL。
  **禁止用 `navigate({ to: '.' })`**（`scripts/check-legacy-frontend-paths.sh` 会拒），用 `useAppNavigation()` 的现有方法或 `navigate({ search: (prev) => ... })`。**执行者需先核实 `useAppNavigation`（`packages/web-core/src/shared/hooks/useAppNavigation.ts`）暴露了哪个适合只改 search 的方法。**
- [ ] **H5.5 跑通** `pnpm run check`。
- [ ] **H5.6 提交** `git add packages/web-core/src/features/kanban/model/kanbanUrlState.* packages/web-core/src/project-routes/project-search.ts packages/web-core/src/features/kanban/ui/KanbanContainer.tsx` / `git commit -m "界面：看板筛选状态写入 URL"`

### H6 · 骨架屏、空态、WIP 提示（7.2 / 7.5）

**Create:** `packages/ui/src/components/Skeleton.tsx`、`packages/ui/src/components/KanbanColumnEmptyState.tsx`
**Modify:** `packages/web-core/src/features/kanban/ui/KanbanContainer.tsx:112-119,890-894,978-981`、`KanbanColumn.tsx`

- [ ] **H6.1 写纯函数测试**：`packages/web-core/src/features/kanban/model/wip.ts` + `.test.ts`
  - `wipState(count, limit)`：`(3,5)` → `'ok'`；`(5,5)` → `'ok'`；`(6,5)` → `'over'`
  - `limit 为 0 或负数时恒为 ok`（关闭提示）
  - `DEFAULT_WIP_LIMIT === 5`
- [ ] **H6.2 实现 `Skeleton`**：`packages/ui/src/components/Skeleton.tsx`
  ```tsx
  export function Skeleton({ className }: { className?: string }) {
    return <div className={cn('animate-pulse rounded bg-panel', className)} />;
  }
  ```
  仓库里目前**没有** Skeleton 组件（已确认），只有散落的 `animate-pulse`（如 `packages/ui/src/components/Table.tsx:83`）。
- [ ] **H6.3 替换加载态**：`KanbanContainer.tsx:112-119` 的 `LoadingState()` 从 `<p>{t('states.loading')}</p>` 改成「3 列 × 每列 4 张卡片」的骨架；`:890-894` 的 `isLoading` 分支保持不变。
- [ ] **H6.4 每列空态与筛选无结果态**：
  - 列为空且无筛选 → `KanbanColumnEmptyState` 显示「新建需求」按钮（调用现有的 `handleAddTask(status.id)`，`KanbanContainer.tsx:812-823`）。
  - 列为空且有筛选（`hasActiveFilters`，:252-268 已有）→ 显示「没有符合筛选的需求 / 清除筛选」，按钮调 `clearKanbanFilters`（:373）。
  - 整板为空 → 保留现有的 `shouldAnimateCreateButton`（:269）并补一条引导文案。
- [ ] **H6.5 列头计数与 WIP**：`KanbanColumn` 的 header 里在状态名后加计数徽标；`wipState === 'over'` 时徽标用 `text-error`，并加 `title={t('kanban.wipHint', {limit})}`。**只提示不阻止。**
- [ ] **H6.6 i18n**：新增 key 写进 7 个 `common.json`（或 `tasks.json`——**用 `tasks` namespace**，因为 `KanbanCardContent.tsx:210` 等已用 `useTranslation('tasks')`）。
- [ ] **H6.7 跑通** `pnpm run check && pnpm run lint`。
- [ ] **H6.8 提交** `git add packages/ui/src/components/Skeleton.tsx packages/ui/src/components/KanbanColumnEmptyState.tsx packages/web-core/src/features/kanban/ packages/web-core/src/i18n/locales/*/tasks.json` / `git commit -m "界面：骨架屏、空列引导与 WIP 提示"`

### H7 · 卡片信息密度与泳道阶段（7.2）

**Modify:** `packages/ui/src/components/KanbanCardContent.tsx:124-141`（props）与 :293-342（第 5 行徽标区）、`KanbanColumn.tsx`

- [ ] **H7.1 写纯函数测试**：`packages/web-core/src/features/kanban/model/cardBadges.ts` + `.test.ts`
  - `buildWorkspaceBadge(workspaces)`：无工作区 → `null`；有 running → `{tone:'running'}`；全部 stopped → `{tone:'stopped'}`；有 failed → `{tone:'failed'}`（**failed 优先级最高**）
  - `buildPrBadge(prs)`：草稿 / 待评审 / 已合并 / 已关闭 四态，**多个 PR 时取「最靠后的状态」**（merged > open > draft），并有测试
  - `buildTestBadge()` 本期恒返回 `{tone:'pending', labelKey:'kanban.testNotConnected'}`（占位）
  - `团队版才显示负责人头像`：`shouldShowAssignees(mode)` → personal 为 false
- [ ] **H7.2 跑确认失败 → 实现 → 跑通**。
- [ ] **H7.3 改视图**：`KanbanCardContent` 的 props 增加 `workspaceBadge?`、`testBadge?`、`showAssignees?`（**有默认值，保证现有调用点不破**）。第 5 行（:293-342）按「工作区 → PR → 测试 → 标签 → 负责人」重排。
  **不要改 `KanbanCardContent` 的现有必选 props**，否则 `KanbanContainer.tsx:1043-1095` 与列表视图都要连锁改。
- [ ] **H7.4 泳道即阶段**：`KanbanColumn` 的 header 增加一个阶段色条（用 `readStageType(status)` + `STAGE_ORDER`），个人版有 `stage_type`、团队版回落成 `'todo'`（H2 已保证不抛错）。
- [ ] **H7.5 跑通 + 提交** `git add packages/ui/src/components/KanbanCardContent.tsx packages/web-core/src/features/kanban/ packages/web-core/src/i18n/locales/*/tasks.json` / `git commit -m "界面：卡片状态徽标与阶段泳道"`

### H8 · 需求详情三段式（7.3）与工作区路径条（7.4）

**Modify:** `packages/ui/src/components/KanbanIssuePanel.tsx:261-544`、`packages/web-core/src/pages/kanban/KanbanIssuePanelContainer.tsx:1041-1112`

现状（§2 已核实）：`KanbanIssuePanel` **没有标签页**，四个 section（Workspaces :526-529、Relationships :531-534、Sub-Issues :536-539、Comments :541-544）是顺序堆叠的 render-prop，全部被 `!isCreateMode && issueId && render*` 包住。

- [ ] **H8.1 写纯函数测试**：`packages/web-core/src/pages/kanban/issuePanelTabs.ts` + `.test.ts`
  - `buildIssuePanelTabs({ mode:'edit' })` → `['overview','development','testing']`
  - `buildIssuePanelTabs({ mode:'create' })` → `['overview']`（新建时不显示开发/测试页）
  - `defaultTab` 在有工作区时是 `'development'`，否则 `'overview'`
  - `tab 值来自 URL 且非法值回落到 overview`
- [ ] **H8.2 跑确认失败 → 实现 → 跑通**。
- [ ] **H8.3 改视图**：在 `KanbanIssuePanel.tsx` 的 Scrollable Content（:298）之下加一层 tab 条（受控，`activeTab` / `onTabChange` 作为**可选 props，默认 `'overview'` 且不渲染 tab 条**，保证 remote-web 等其它调用方不受影响）。
  - `overview` = 现有的 Property Row（:300）、Tags Row（:321）、Title/Description（:335-487）、Relationships、Sub-Issues、Comments
  - `development` = Workspaces Section（:526-529）+ PR 链接
  - `testing` = 新的占位面板：`<p>{t('kanban.testingNotConnected')}</p>` + 预留「通过率 / 失败清单 / 报告链接」三个空槽（**只写结构，不接数据**）
- [ ] **H8.4 工作区路径条（7.4）**：在 `packages/ui/src/components/IssueWorkspaceCard.tsx` 之外新建 `packages/ui/src/components/WorkspaceBreadcrumb.tsx`，展示「需求 simple_id → 工作区名 → PR #n」，每段可点。挂在 `packages/web-core/src/pages/workspaces/WorkspacesMainContainer.tsx`（273 行，`export const WorkspacesMainContainer = forwardRef<...>` 在 :106）的顶部。
  纯函数 `buildWorkspaceBreadcrumb(...)` + 测试：无关联需求时只有两段；无 PR 时只有两段；全有时三段。
- [ ] **H8.5 跑通** `pnpm run check && pnpm run lint`。
- [ ] **H8.6 提交** `git add packages/ui/src/components/KanbanIssuePanel.tsx packages/ui/src/components/WorkspaceBreadcrumb.tsx packages/web-core/src/pages/kanban/ packages/web-core/src/pages/workspaces/WorkspacesMainContainer.tsx packages/web-core/src/i18n/locales/*/tasks.json` / `git commit -m "界面：需求详情三段式与工作区路径条"`

### H9 · 快捷键补齐与帮助弹窗（7.5）

**Modify:** `packages/web-core/src/shared/keyboard/useIssueShortcuts.ts`、`packages/web-core/src/shared/dialogs/shared/KeyboardShortcutsDialog.tsx`

现状（已核实 `packages/web-core/src/shared/keyboard/registry.ts:372-455`）——**设计文档 §7.5 列的快捷键大部分已经存在，只是绑的键不同或没接到看板上**：

| 设计文档要求 | 仓库现状 | 依据 | 本步要做的 |
|---|---|---|---|
| `/` 搜索 | 已有 `Action.FOCUS_SEARCH` 绑 `slash`，scope `KANBAN` | registry.ts:398-404 | 只需在 `KanbanContainer` 里调 `useKeyFocusSearch()` 并 focus 到 `KanbanFilterBar` 的搜索框 |
| `n` 新建需求 | 已有 `Action.CREATE` 绑 **`c`**（不是 `n`） | registry.ts:373-379 | **决定：保留 `c`，不改键位**（改键会破坏老用户肌肉记忆），在帮助弹窗里如实显示 `c`；设计文档这一条按「已满足」处理 |
| `j/k` 上下 | 已有 `NAV_UP`=`k`、`NAV_DOWN`=`j`，scope `KANBAN` | registry.ts:405-417 | 接上 `cardNavigation.ts` 的纯函数 |
| `Enter` 打开 | 现状是 `meta+enter` / `ctrl+enter`（`OPEN_DETAILS`） | registry.ts:434-441 | **新增**一条 `keys: 'enter'`、scope `KANBAN` 的绑定（与 `DIALOG` scope 的 `SUBMIT`=`enter`（:389-395）不冲突，因为 scope 不同） |
| `e` 编辑 | **没有** | — | 新增 `Action.EDIT`（registry 里加枚举值 + 定义） |
| `Esc` 关闭面板 | 已有 `escape` 清空多选（`useIssueShortcuts.ts:199-210`） | — | **不要新增第二个 escape 绑定**，改成在同一个 handler 里分级处理：有多选先清多选，否则关闭右侧面板 |
| `?` 帮助 | 已有 `Action.SHOW_HELP` 绑 `shift+slash`，scope `GLOBAL` | registry.ts:449-455 | 确认 `KeyboardShortcutsDialog` 里能列出新增项即可 |

- [ ] **H9.1 写纯函数测试**：`packages/web-core/src/features/kanban/model/cardNavigation.ts` + `.test.ts`
  - `nextIssueId(ids, current, 'down')`：末尾时停在末尾（不回绕）
  - `nextIssueId(ids, null, 'down')` → 第一个
  - `nextIssueId([], null, 'down')` → `null`
  - `上下移动跨列时的顺序取自 orderedIssueIds`（`KanbanContainer.tsx:766-788` 已有该数组）
- [ ] **H9.2 跑确认失败 → 实现 → 跑通**。
- [ ] **H9.3 按上表接线**：新增的 `Action.EDIT` 与 `enter` 绑定写进 `registry.ts`；在 `KanbanContainer` 里用 `hooks.ts` 的语义 hook（`useKeyFocusSearch` :52、`useKeyNavUp/Down` :57-58、`useKeyOpenDetails` :68、`useKeyCreate` :35）接上。
  **务必不要直接 `useHotkeys('j', ...)`**——绕开 registry 会让帮助弹窗漏项，也会和既有绑定打架。
- [ ] **H9.4 跑通 + 提交** `git add packages/web-core/src/shared/keyboard/ packages/web-core/src/shared/dialogs/shared/KeyboardShortcutsDialog.tsx packages/web-core/src/features/kanban/model/cardNavigation.* packages/web-core/src/i18n/locales/*/common.json` / `git commit -m "界面：看板快捷键与帮助弹窗"`

### H10 · 密度切换与信息架构（7.1 / 7.5）

**Modify:** `packages/web-core/src/shared/stores/useUiPreferencesStore.ts`（现 1017 行）、`packages/ui/src/components/KanbanFilterBar.tsx`

- [ ] **H10.1 写纯函数测试**：`densityClasses(density)` → `'comfortable'` / `'compact'` 两套 class 字符串；非法值回落 `'comfortable'`。
- [ ] **H10.2 实现**：在 `useUiPreferencesStore` 里加 `kanbanDensity: 'comfortable' | 'compact'` 与 `setKanbanDensity`。
  **纠正设计文档 §7.5「记在本地存储」：该 store 并没有用 zustand 的 persist 中间件**（`useUiPreferencesStore.ts:448` 是裸 `create<State>()(...)`，全文件没有 `persist(`），持久化走的是**服务端 scratch 表**（`useUiPreferencesStore.ts:353` 注释「persisted via scratch store」，Rust 侧结构在 `crates/db/src/models/scratch.rs:101-125` 的 `UiPreferencesData`）。唯一用 `localStorage` 的是移动端字号（:32,829-834）。
  → **决定：密度跟随既有机制放进 scratch**。需要在 `crates/db/src/models/scratch.rs` 的 `UiPreferencesData` 里加
  ```rust
  /// 看板卡片密度：comfortable / compact
  #[serde(default)]
  pub kanban_density: Option<String>,
  ```
  然后 `pnpm run generate-types`（`UiPreferencesData` 已注册在 `crates/server/src/bin/generate_types.rs`）。加 `#[serde(default)]` 保证老数据能反序列化——写一条 Rust 测试 `旧的_UiPreferences_JSON_仍能反序列化`（用一段不含 `kanban_density` 的 JSON）。
- [ ] **H10.3 左侧导航（7.1）**：现有布局是 `SharedAppLayout`（`packages/web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx`，由 `packages/local-web/src/routes/_app.tsx:23` 引入）。
  **本期只做增量**：在现有导航里加「测试」占位项（点击进入一个说明页）与团队版的「成员」项（G7 已加路由）。
  **不要重排现有导航结构**，避免连锁破坏 8 个路由文件。
- [ ] **H10.4 跑通** `pnpm run check && pnpm run lint`。
- [ ] **H10.5 提交** `git add packages/web-core/src/shared/stores/useUiPreferencesStore.ts packages/ui/src/components/KanbanFilterBar.tsx packages/web-core/src/shared/components/ui-new/containers/SharedAppLayout.tsx packages/web-core/src/i18n/locales/*/common.json` / `git commit -m "界面：密度切换与导航补齐"`

**任务 H 验收：**
```
cargo test -p db stage
pnpm --filter @vibe/web-core test
pnpm run check
pnpm run lint
```
人工验收（设计文档 §11.7）：
- 浅色 + 暗色两种主题下逐屏看一遍，无低对比度文字（`text-low` 在 `bg-panel` 上要能读）。
- 浏览器窗口宽 1280px 时看板区不出现横向滚动条（列宽自适应）。
- 个人版下看不到任何头像 / 指派 / 成员入口。

---

## 任务 I：部署

**目标：** launchd / systemd 模板与安装脚本、备份与恢复、`local-build.sh` 参数化、`pnpm run build:selfhost`。
**依赖：** A（模式）、F（WAL 与备份）。

### I1 · 配置样例与数据目录说明

**Create:** `deploy/server.example.json`

- [ ] **I1.1 写文件**：
  ```json
  {
    "mode": "team",
    "allow_oauth_signup": false,
    "session_ttl_days": 30,
    "sqlite_wal": true,
    "trust_proxy": false,
    "public_base_url": "http://192.168.1.10:8080",
    "oauth": {
      "feishu":  { "client_id": "", "client_secret": "" },
      "lark":    { "client_id": "", "client_secret": "" },
      "google":  { "client_id": "", "client_secret": "" }
    }
  }
  ```
- [ ] **I1.2 加一条 Rust 测试**（`crates/services/src/services/server_settings.rs`）：
  `样例配置文件可以被解析`：`include_str!("../../../../deploy/server.example.json")` → `serde_json::from_str::<ServerSettingsFile>()` 成功，且 `mode == Team`。
  **这条测试把「文档里的样例」和「代码里的结构」绑在一起**，样例写错会被 CI 抓到。
  路径：从 `crates/services/src/services/server_settings.rs` 到仓库根是 `../../../..`（`crates/services/src/services/` → `crates/services/src/` → `crates/services/` → `crates/` → 根）。**执行者先用 `cargo test` 确认层数**，错了会是编译期错误，好定位。
- [ ] **I1.3 提交** `git add deploy/server.example.json crates/services/src/services/server_settings.rs` / `git commit -m "部署：server.json 样例与解析测试"`

### I2 · macOS launchd

**Create:** `deploy/launchd/ai.bloop.vibe-kanban.plist`、`deploy/install-macos.sh`

- [ ] **I2.1 写 plist**（占位符 `__BIN_PATH__`、`__DATA_DIR__`、`__PORT__` 由安装脚本替换）：
  ```xml
  <?xml version="1.0" encoding="UTF-8"?>
  <!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
  <plist version="1.0">
  <dict>
    <key>Label</key><string>ai.bloop.vibe-kanban</string>
    <key>ProgramArguments</key><array><string>__BIN_PATH__</string></array>
    <key>EnvironmentVariables</key>
    <dict>
      <key>VK_MODE</key><string>team</string>
      <key>HOST</key><string>0.0.0.0</string>
      <key>BACKEND_PORT</key><string>__PORT__</string>
      <key>VK_ALLOWED_ORIGINS</key><string>__ALLOWED_ORIGINS__</string>
    </dict>
    <key>RunAtLoad</key><true/>
    <key>KeepAlive</key><true/>
    <key>StandardOutPath</key><string>__DATA_DIR__/logs/stdout.log</string>
    <key>StandardErrorPath</key><string>__DATA_DIR__/logs/stderr.log</string>
    <key>WorkingDirectory</key><string>__DATA_DIR__</string>
  </dict>
  </plist>
  ```
  `KeepAlive=true` 就是设计文档要求的 `Restart=always` 等价物。
- [ ] **I2.2 写 `deploy/install-macos.sh`**：`set -euo pipefail`；参数 `--bin`、`--port`、`--origins`；
  - 计算数据目录 `${HOME}/Library/Application Support/ai.bloop.vibe-kanban`，`mkdir -p "$DATA_DIR/logs"`
  - `sed` 替换占位符后写到 `~/Library/LaunchAgents/ai.bloop.vibe-kanban.plist`
  - `launchctl bootout gui/$(id -u)/ai.bloop.vibe-kanban 2>/dev/null || true`，再 `launchctl bootstrap gui/$(id -u) <plist>`
  - 末尾 `launchctl print gui/$(id -u)/ai.bloop.vibe-kanban | head -20` 打印状态
  - **脚本必须在替换前校验 `--bin` 指向的文件存在且可执行**，否则 launchd 会静默不断重启。
- [ ] **I2.3 `chmod +x deploy/install-macos.sh`**，跑 `bash -n deploy/install-macos.sh`（语法检查）与 `shellcheck deploy/install-macos.sh`（若可用）。
- [ ] **I2.4 提交** `git add deploy/launchd/ deploy/install-macos.sh` / `git commit -m "部署：macOS launchd 模板与安装脚本"`

### I3 · Linux systemd

**Create:** `deploy/systemd/vibe-kanban.service`、`deploy/install-linux.sh`

- [ ] **I3.1 写 unit**：
  ```ini
  [Unit]
  Description=Vibe Kanban (team mode)
  After=network-online.target
  Wants=network-online.target

  [Service]
  Type=simple
  ExecStart=__BIN_PATH__
  Environment=VK_MODE=team
  Environment=HOST=0.0.0.0
  Environment=BACKEND_PORT=__PORT__
  Environment=VK_ALLOWED_ORIGINS=__ALLOWED_ORIGINS__
  WorkingDirectory=__DATA_DIR__
  Restart=always
  RestartSec=3
  StandardOutput=append:__DATA_DIR__/logs/stdout.log
  StandardError=append:__DATA_DIR__/logs/stderr.log
  NoNewPrivileges=true
  PrivateTmp=false

  [Install]
  WantedBy=default.target
  ```
  **`PrivateTmp=false` 是必须的**：MCP 通过 `std::env::temp_dir()/vibe-kanban/vibe-kanban.port` 找服务端口（`crates/utils/src/port_file.rs:18-19`），开了 `PrivateTmp` 就找不到。这一条要写进注释。
- [ ] **I3.2 写 `deploy/install-linux.sh`**：装到 `~/.config/systemd/user/`，`systemctl --user daemon-reload && systemctl --user enable --now vibe-kanban`；数据目录 `${XDG_DATA_HOME:-$HOME/.local/share}/vibe-kanban`。
  **提示用户跑 `loginctl enable-linger $USER`**，否则注销后服务会停。
- [ ] **I3.3 `bash -n` + 提交** `git add deploy/systemd/ deploy/install-linux.sh` / `git commit -m "部署：Linux systemd 模板与安装脚本"`

### I4 · 备份与恢复

**Create:** `deploy/backup.sh`、`deploy/restore.sh`

- [ ] **I4.1 写 `deploy/backup.sh`**：
  ```bash
  #!/usr/bin/env bash
  set -euo pipefail
  DATA_DIR="${VK_DATA_DIR:?必须设置 VK_DATA_DIR}"
  OUT_DIR="${1:-$DATA_DIR/backups}"
  KEEP="${VK_BACKUP_KEEP:-7}"
  command -v sqlite3 >/dev/null || { echo "缺少 sqlite3 命令"; exit 1; }
  mkdir -p "$OUT_DIR"
  STAMP="$(date +%Y%m%d-%H%M%S)"
  WORK="$(mktemp -d)"
  trap 'rm -rf "$WORK"' EXIT
  # 一致性备份：WAL 模式下直接 cp 会丢 -wal 里未 checkpoint 的数据
  sqlite3 "$DATA_DIR/db.v2.sqlite" ".backup '$WORK/db.v2.sqlite'"
  # 其余配置文件按原样打包（不含 credentials.json 里的云端令牌？—— 见下）
  for f in server.json config.json profiles.json; do
    [ -f "$DATA_DIR/$f" ] && cp "$DATA_DIR/$f" "$WORK/"
  done
  tar -czf "$OUT_DIR/vibe-kanban-$STAMP.tar.gz" -C "$WORK" .
  ls -1t "$OUT_DIR"/vibe-kanban-*.tar.gz | tail -n +$((KEEP+1)) | xargs -r rm -f
  echo "备份完成：$OUT_DIR/vibe-kanban-$STAMP.tar.gz"
  ```
  **刻意不备份 `credentials.json`、`machine_token`、`server_ed25519_signing_key`**（`crates/utils/src/assets.rs:41,45,49`）：它们是本机凭据，打进备份包会扩大泄露面。文档里写明「恢复后需要重新登录云端 / 重新配对 relay」。
- [ ] **I4.2 写 `deploy/restore.sh`**：解包到临时目录 → 校验 `db.v2.sqlite` 存在且 `sqlite3 <db> "PRAGMA integrity_check"` 返回 `ok` → **提示先停服务**（检测端口文件是否存在且进程在跑，在跑就拒绝执行并提示命令）→ 把现有数据目录里的 `db.v2.sqlite`（含 `-wal`、`-shm`）先改名成 `.bak.<stamp>` → 拷回。
- [ ] **I4.3 写脚本测试**（bash，放 `deploy/backup_test.sh`，由 J3 接进 CI 或手工跑）：
  - 建一个临时数据目录 + 用 `sqlite3` 造一张表 → 跑 backup → 解包 → 断言数据在
  - **WAL 场景**：`PRAGMA journal_mode=WAL` 后写数据不 checkpoint → backup → 断言数据在（对照组：直接 `cp` 主库文件 → 数据不在）
  - `保留份数生效`：连跑 9 次，断言目录里只有 7 个包
  - `缺少 sqlite3 时明确报错`（用 `PATH=` 屏蔽）
- [ ] **I4.4 `chmod +x` + `bash -n` + 提交** `git add deploy/backup.sh deploy/restore.sh deploy/backup_test.sh` / `git commit -m "部署：基于 sqlite3 .backup 的备份与恢复脚本"`

### I5 · 局域网与 HTTPS 样例

**Create:** `deploy/Caddyfile.selfhost.example`

- [ ] **I5.1 写样例**（仓库根已有一个 `Caddyfile.example`，是开发用的 relay 反代，**不要覆盖它**）：
  ```
  kanban.example.com {
      encode gzip
      reverse_proxy 127.0.0.1:8080 {
          header_up X-Forwarded-For {remote_host}
          header_up X-Forwarded-Proto {scheme}
      }
  }
  ```
  配套说明写进 J1：此时必须
  - `VK_TRUST_PROXY=1`（否则限速按代理 IP 分桶，一个人被封所有人被封）
  - `VK_PUBLIC_BASE_URL=https://kanban.example.com`（Cookie 才会带 `Secure`，OAuth 回调才正确）
  - `VK_ALLOWED_ORIGINS=https://kanban.example.com`
- [ ] **I5.2 提交** `git add deploy/Caddyfile.selfhost.example` / `git commit -m "部署：自建 HTTPS 反代样例"`

### I6 · `local-build.sh` 参数化与 `build:selfhost`

**Modify:** `local-build.sh:46-47`、`package.json`（scripts）

- [ ] **I6.1 改 `local-build.sh:46-47`**：
  ```bash
  # 云端基址：默认空＝纯本地，不连任何云端。
  # 官方发布包由 CI 显式传入 VK_SHARED_API_BASE=https://api.vibekanban.com。
  export VK_SHARED_API_BASE="${VK_SHARED_API_BASE:-}"
  export VITE_VK_SHARED_API_BASE="${VITE_VK_SHARED_API_BASE:-$VK_SHARED_API_BASE}"
  ```
  **风险与对策（已核实一半）：** 这会改变默认行为（原来硬编码指向官方云端）。
  - `.github/workflows/pre-release.yml` **已经**在多处注入 `VK_SHARED_API_BASE: ${{ secrets.VK_SHARED_API_BASE }}`（:350、:366、:385、:917）和 `VITE_VK_SHARED_API_BASE`（:168），所以它不受影响。
  - `.github/workflows/publish.yml` grep `build:npx` / `local-build` / `VK_SHARED_API_BASE` **零命中**，说明它可能不走 `local-build.sh`。**执行者必须先通读 `publish.yml` 确认它的构建路径**；若它确实调用 `local-build.sh`（间接也算），必须补 `env: VK_SHARED_API_BASE: ${{ secrets.VK_SHARED_API_BASE }}`。写进 §12。
  - 本 fork 是自建场景，即便官方包行为变化也不影响本仓库的使用；但为了不给上游制造 breaking change，仍按上面处理。
- [ ] **I6.2 加 `pnpm run build:selfhost`**：`package.json` 的 scripts 里新增
  ```json
  "build:selfhost": "VK_SHARED_API_BASE= bash ./scripts/build-selfhost.sh"
  ```
  **Create:** `scripts/build-selfhost.sh`：
  - 跑 `bash ./local-build.sh`（此时 `VK_SHARED_API_BASE` 为空）
  - 产出目录 `dist-selfhost/`：可执行文件（从 `${CARGO_TARGET_DIR:-target}/release/server` 拷成 `vibe-kanban`）+ `deploy/` 整个目录 + 一个 `README.md`（安装三步走）
  - `tar -czf dist-selfhost/vibe-kanban-selfhost-<platform>.tar.gz ...`
  - **加 `dist-selfhost/` 到 `.gitignore`**
- [ ] **I6.3 验证**：`bash -n scripts/build-selfhost.sh`；本机若愿意可实跑一次（耗时较长，release 编译）。
- [ ] **I6.4 提交** `git add local-build.sh package.json scripts/build-selfhost.sh .gitignore .github/workflows/publish.yml .github/workflows/pre-release.yml` / `git commit -m "部署：构建参数化与自建部署包"`

**任务 I 验收：**
```
bash -n deploy/install-macos.sh deploy/install-linux.sh deploy/backup.sh deploy/restore.sh scripts/build-selfhost.sh
bash deploy/backup_test.sh          # 预期全部 PASS
cargo test -p services server_settings   # 含样例配置解析测试
```

---

## 任务 J：文档与门禁

**依赖：** A～I 全部。

### J1 · 部署与运维文档

**Create:** `docs/self-hosting/team-mode.mdx`
**Modify:** `docs/docs.json:104-108`

- [ ] **J1.1 写文档**：遵循 `docs/CLAUDE.md` 的 Mintlify 规范（必须有 `title` / `description` frontmatter；用 `<Steps>` / `<Warning>` / `<Tabs>` 组件；British English）。内容至少覆盖：
  - 两种模式的启动命令与区别
  - 首启初始化向导（一次性令牌 30 分钟）
  - `server.json` 全部字段（表格，与 §1.3 一致）
  - 局域网访问：`VK_ALLOWED_ORIGINS` 必须配成成员实际访问的来源
  - HTTPS：Caddy 十行配置 + `VK_TRUST_PROXY=1` + `VK_PUBLIC_BASE_URL`
  - `<Warning>`：明文 HTTP 下 Cookie 无 `Secure`，只建议在受信任内网使用；**不要暴露到公网**
  - `<Warning>`：团队模式不支持云端 relay 远程访问（§3.6）
  - 三个 OAuth 提供方的配置步骤与回调 URL
- [ ] **J1.2 改导航**：`docs/docs.json` 的 Self-Hosting 分组（:103-108）里加 `"self-hosting/team-mode"`。
- [ ] **J1.3 提交** `git add docs/self-hosting/team-mode.mdx docs/docs.json` / `git commit -m "文档：团队版部署指南"`

### J2 · 备份恢复与迁移说明

**Create:** `docs/self-hosting/backup-restore.mdx`

- [ ] **J2.1 写文档**：备份脚本用法、恢复流程「停服务 → 恢复 → 起服务」、保留份数、**为什么不能直接 cp**（WAL）、恢复后需要重新配置的本机凭据清单。
- [ ] **J2.2 迁移说明**：单独一节写「从个人版升级到团队版」：
  - 升级是纯加法：新增 4 张表 + 1 条固定用户行，历史需求的 `creator_user_id` 不改写（有测试，见 B1）
  - 切 team 模式前先备份
  - 切回 personal 模式不丢数据（模式只影响鉴权，不影响数据）
  - `VK_SQLITE_WAL=0` 可以回退 journal 模式
- [ ] **J2.3 改导航 + 提交** `git add docs/self-hosting/backup-restore.mdx docs/docs.json` / `git commit -m "文档：备份恢复与升级说明"`

### J3 · CI 门禁补齐

**Modify:** `.github/workflows/test.yml:178-195`

- [ ] **J3.1 把 web-core 的 vitest 接进 CI**：在 `frontend-checks` 的 `concurrently` 列表里增加一项
  ```
  "cd packages/web-core && npm run test" \
  ```
  并在 `--names` 里加 `core:test`。
  **现状：CI 只跑 `packages/web-core && npm run check`（tsc），不跑 vitest**（§2.8 已核实），所以前端单测今天在 CI 里是没有把门的。
- [ ] **J3.2 把 deploy 脚本检查接进 CI**（可选但推荐）：加一步
  ```yaml
  - name: Shell syntax
    run: bash -n deploy/*.sh scripts/build-selfhost.sh
  ```
- [ ] **J3.3 改 paths-filter**：`.github/workflows/test.yml:44-59` 的 `frontend` 过滤器加 `'deploy/**'`？——不，`deploy/**` 属于后端/部署，另开一个 filter 或直接放进 `backend` 列表（:60-100）。**决定：加到 `backend` 列表里**（第 :90 `- 'Cargo.toml'` 附近插 `- 'deploy/**'`）。
- [ ] **J3.4 提交** `git add .github/workflows/test.yml` / `git commit -m "门禁：CI 增加前端单测与部署脚本语法检查"`

### J4 · 全量门禁

- [ ] **J4.1 格式化**（`CLAUDE.md` 明确要求「完成任务前跑 `pnpm run format`」）
  `pnpm run format`
  预期：`cargo fmt` + `cargo fmt --manifest-path crates/remote/Cargo.toml` + 3 个 prettier 全部成功。
  然后 `git status` 应干净或只有格式化改动；有改动就 `git add -A && git commit -m "格式化"`。
- [ ] **J4.2 全量测试**
  ```
  cargo test --workspace
  pnpm run check
  pnpm run lint
  pnpm --filter @vibe/web-core test
  pnpm run generate-types:check
  pnpm run prepare-db:check
  GITHUB_BASE_REF=main ./scripts/check-i18n.sh
  ./scripts/check-legacy-frontend-paths.sh
  ```
  **逐条确认输出，不允许「大概过了」。** 任何一条失败就回到对应任务修复。
- [ ] **J4.3 手工验收设计文档 §11 的 7 条**，逐条打勾：
  1. 个人版：`cargo run --bin server` 直接启动，免登录，建项目 → 建需求 → 拖拽 → 创建工作区 → 合并 → 需求完成全流程照常
  2. 团队版：`VK_MODE=team HOST=0.0.0.0 BACKEND_PORT=8080` → 初始化管理员 → 建 2 个成员 → 三个浏览器分别登录 → 各自建需求、互相可见 → 一侧改动另一侧 2 秒内更新（**注意是 WebSocket 通道，不是 SSE**）
  3. 未登录访问任意 `/api/*` 返回 401；伪造 Cookie、过期会话、被停用用户、跨站请求全部被拒（自动化测试已覆盖，这里只做抽样复核）
  4. 三个 OAuth 提供方在 mock IdP 下走通；未配置凭据时登录页不出现按钮
  5. 部署包在一台干净机器（或一个干净的 `VK_DATA_DIR`）上：解压 → 跑安装脚本 → 服务自启 → 浏览器可访问；备份恢复可用
  6. 全量门禁绿（J4.2）
  7. 暗色 / 浅色对比度、1280px 无横向滚动（H 任务验收项）
- [ ] **J4.4 开 PR**
  ```
  git push -u origin team-mode-and-ui
  gh pr create --base main --title "团队版、本地认证与界面优化（P3）" --body-file <(...)
  ```
  PR 描述至少包含：范围、**不做的事**（设计文档 §3 的「本期不做」清单 + §3.6 的 relay 限制）、迁移影响、安全评审要点（§3 全部条目）、验收记录。

---

## 10. 设计文档覆盖表

| 设计文档小节 | 内容 | 覆盖任务 | 备注 |
|---|---|---|---|
| §3 范围 1 | `local_users` / `local_sessions` / `local_user_identities` | A2、A3、A4 | 另加 `local_invites` |
| §3 范围 2 | Argon2id + HttpOnly Cookie；登录、登出、改密、首启建管理员 | A5、A6、A9、D4 | |
| §3 范围 3 | 飞书 / Lark / Google OAuth2，未配置则隐藏 | E1、E3、A9（bootstrap 返回 providers）、G5 | 端点 URL 待真实核实 |
| §3 范围 4 | `personal` / `team` 一个二进制两模式 | A1、A7、A8 | |
| §3 范围 5 | 成员管理 + 邀请链接注册 | D2、D3 | |
| §3 范围 6 | 身份落地，个人版固定用户，历史数据不受影响 | B1～B5 | B1 有专门的回归测试 |
| §3 范围 7 | 鉴权覆盖 `/api/*` 与实时通道；写操作 CSRF | A9（装配）、C1、C2 | **实时通道是 WS 不是 SSE**，见 §3.4 |
| §3 范围 8 | 界面优化 | H1～H10 | |
| §3 范围 9 | 部署：启动脚本、launchd/systemd、首启向导、备份恢复、局域网与 HTTPS | I1～I6、D4、J1、J2 | |
| §5 数据模型 | 四张表 + 字段 | A2 | 字段与设计文档一致，另加 `local_invites.created_at` |
| §5 说明 1 | Argon2id m=19456,t=2,p=1 | A5.2 有断言测试 | |
| §5 说明 2 | 32 字节令牌、存 SHA-256、30 天、滑动续期、多设备 | A6、A4（`touch_last_seen` 节流） | 「滑动续期」实现为「更新 `last_seen_at`」；**`expires_at` 不延长**（见 §12） |
| §5 说明 3 | 固定用户写入 `local_users` | A2.3 | |
| §5 说明 4 | 迁移不改写历史 `creator_user_id`/`author_id` | B1 | |
| §6.1 模式 | 环境变量 + `server.json`，前端从 bootstrap 读 | A1、G1、G4 | |
| §6.2 会话 | Cookie 属性、中间件、节流、登出、改密撤销、401 不区分、限速 | A6、A8、A9.4、C3 | **SameSite 改 Lax**，见 §3.2 |
| §6.3 CSRF | SameSite + Origin 强校验 + 双提交 | C1、C2 | |
| §6.4 第三方登录 | 抽象、三份配置、state 一次性、不自动合并、不记 token、mock IdP | E1、E2、E3 | |
| §6.5 权限 | admin / member；全员可见；删除仅 admin 或本人 | D2 | **「删除项目 / 删除他人评论仅 admin 或本人」未覆盖**，见下表 |
| §7.1 信息架构 | 顶栏、左侧导航、主区、右侧栏 | H10.3（增量）、G6 | **未做整体重排**，只加「测试」与「成员」两项，理由见 H10.3 |
| §7.2 看板 | 泳道即阶段（+test）、卡片密度、列头计数与 WIP、拖拽前置条件确认、空列引导 | H1、H2、H6、H7 | **「拖拽落列前置条件确认」未覆盖**，见下表 |
| §7.3 需求详情三段式 | 概况 / 开发 / 测试 | H8 | |
| §7.4 工作区 | 列表按需求分组；详情顶部路径条 | H8.4 | **「工作区列表按需求分组」未覆盖**，见下表 |
| §7.5 交互细节 | 快捷键、URL 筛选、密度、骨架屏、空态、设计令牌、暗色、团队版元素隐藏 | H5、H6、H9、H10、G6 | |
| §8.1～8.6 部署 | 形态、启动、服务化、备份恢复、局域网 HTTPS、构建 | I1～I6、J1、J2 | |
| §9 并发与数据安全 | WAL + synchronous、钩子验证、Windows 锁、备份用 .backup、连接池 16、db_retry 保留、会话清理 | F1、F2、F3、C4 | **Windows 文件锁未验证**（本机 macOS），见 §12 |
| §10 风险与对策 | 7 条 | 分散在 A/C/E/F/H | |
| §11 验收标准 | 7 条 | J4.3 | |

### 10.1 明确**不覆盖**的项与理由

| 未覆盖项 | 出处 | 理由 |
|---|---|---|
| 删除项目 / 删除他人评论的细粒度权限 | §6.5 末句 | 现有删除路由（`local_projects/projects.rs:110` `delete_project`、`side.rs` 的评论删除）没有作者字段校验。补这个需要改 `IssueComments::delete` 的签名并加「作者或 admin」判断。**本期不做**，因为团队内全员可信且设计文档 §3 的「本期不做」已排除可见性隔离；若要做，是任务 D 的一个追加步骤，工作量约 1 小时。**必须在 PR 描述里写明这是已知缺口。** |
| 拖拽落列的前置条件确认（「待评审」要求有工作区） | §7.2 第 4 点 | `handleDragEnd`（`KanbanContainer.tsx:658-750`）是纯乐观更新 + `bulkUpdateIssues`，插入一次确认对话框需要把它改成 async 流程并处理取消回滚，风险高于收益。**本期不做**，列头的 WIP 提示（H6.5）已经提供了同类的「提示不阻止」体验。 |
| 工作区列表按需求分组 | §7.4 第 1 点 | `WorkspacesSidebarContainer.tsx`（707 行）已有 `WorkspaceLayoutMode = 'flat' \| 'accordion'`（:57），改成按需求分组要动这 707 行的分组逻辑。**本期只做路径条（H8.4）**，分组留给下一期。 |
| 信息架构整体重排（§7.1 的四区布局） | §7.1 | 现有布局由 `SharedAppLayout` + `_app.tsx` + `ProjectKanban.tsx` 的 `react-resizable-panels` 共同决定，重排会连锁影响 8 个路由文件。按「逐屏改造不推倒重来」的原则，**本期只做增量**（H10.3）。 |
| Windows 下 WAL 文件锁验证 | §9 | 本机 macOS，无 Windows 环境。**必须在合并前由有 Windows 的人验证一次**，否则保留 `VK_SQLITE_WAL=0` 作为逃生口。 |
| 测试结果回填（atp `results.json`） | §3「本期不做」 | 设计文档已排除；H8.3 只留结构占位。 |
| 企业 SSO、端到端加密、审计日志、多租户计费 | §3「本期不做」 | 设计文档已排除。 |

---

## 11. 跨任务一致性核对表

执行完全部任务后，逐条 `grep` 核对（任何一条对不上就是 bug）：

### 11.1 路由路径

| 字符串 | 必须一致出现在 |
|---|---|
| `/api/local-auth/bootstrap` | `crates/server/src/routes/local_auth/mod.rs`、`packages/web-core/src/shared/lib/local/bootstrapApi.ts` |
| `/api/local-auth/login` | 同上 |
| `/api/local-auth/logout` | 同上 |
| `/api/local-auth/me` | 同上 |
| `/api/local-auth/password` | 同上 |
| `/api/local-auth/setup` | `setup.rs`、登录页容器（`?setup=` 参数）、`main.rs` 打印的链接 |
| `/api/local-auth/invites/accept` | `local_auth/mod.rs`、注册页容器 |
| `/api/local-auth/oauth/{provider}/start` / `/callback` | `oauth_routes.rs`、`oauth.rs` 的 `redirect_uri` 拼接、登录页的跳转 URL、`deploy/server.example.json` 的注释、`docs/self-hosting/team-mode.mdx` |
| `/api/admin/users` / `/api/admin/invites` | `admin/users.rs`、`admin/invites.rs`、`MembersPageContainer.tsx` |
| `/api/health` | `routes/mod.rs`、公共路由清单测试 |
| 核对命令 | `grep -rn "local-auth" crates packages docs deploy \| sort` 后人工比对 |

### 11.2 Cookie / 请求头名

| 名称 | Rust 常量 | TS 使用点 |
|---|---|---|
| `vk_session` | `services::…::token::SESSION_COOKIE` | 无（HttpOnly，前端不读） |
| `vk_csrf` | `services::…::token::CSRF_COOKIE` | `packages/web-core/src/shared/lib/local/csrf.ts` |
| `X-VK-CSRF` | `services::…::token::CSRF_HEADER`（小写 `x-vk-csrf`） | `localApiTransport.ts` 里的 `headers.set('X-VK-CSRF', ...)` |
| `X-VK-MACHINE-TOKEN` | `middleware/local_session.rs` 常量 | `crates/mcp/src/task_server/mod.rs` |
| 核对命令 | `grep -rn "vk_session\|vk_csrf\|VK-CSRF\|MACHINE-TOKEN" crates packages` |

**易错点：HTTP 头名在 axum 里是小写的**（`HeaderMap` 用 `HeaderName`，比较时不区分大小写但 `get("X-VK-CSRF")` 与 `get("x-vk-csrf")` 都可以）。Rust 侧常量统一写小写，TS 侧写什么都行。

### 11.3 环境变量名

`VK_MODE`、`VK_ALLOW_OAUTH_SIGNUP`、`VK_SESSION_TTL_DAYS`、`VK_SQLITE_WAL`、`VK_TRUST_PROXY`、`VK_PUBLIC_BASE_URL`、`VK_OAUTH_{FEISHU,LARK,GOOGLE}_CLIENT_{ID,SECRET}`

必须一致出现在：`server_settings.rs`、`crates/db/src/lib.rs`（仅 `VK_SQLITE_WAL`）、`deploy/*.plist` / `*.service`、`deploy/server.example.json`（对应 JSON 字段）、`docs/self-hosting/team-mode.mdx`。
核对命令：`grep -rohn "VK_[A-Z_]*" crates deploy docs scripts package.json | sort -u`

### 11.4 数据库列名 ↔ Rust 字段 ↔ TS 字段

| SQLite 列 | Rust 字段 | TS 字段（`shared/types.ts`） |
|---|---|---|
| `local_users.username` | `LocalUser.username` | `LocalAuthUser.username` |
| `local_users.display_name` | `LocalUser.display_name` | `LocalAuthUser.display_name` |
| `local_users.avatar_color` | `LocalUser.avatar_color` | `LocalAuthUser.avatar_color` |
| `local_users.role` | `LocalUser.role: LocalUserRole` | `LocalAuthUser.role: string` |
| `local_users.status` | `LocalUser.status: LocalUserStatus` | 不暴露给普通用户接口，只在 admin 接口里 |
| `local_sessions.token_hash` | **不出现在任何结构体里** | 不出现 |
| `local_user_identities.subject` | `LocalUserIdentity.subject` | admin/设置页可见 |
| `project_statuses.stage_type` | `LocalProjectStatus.stage_type: String` | **不在 `shared/remote-types.ts` 的 `ProjectStatus` 里**，前端用 `stageType.ts` 的本地扩展类型读 |

核对命令：`pnpm run generate-types:check`（Rust ↔ TS）、`pnpm run prepare-db:check`（Rust ↔ SQLite）。

### 11.5 枚举值字符串

| 概念 | 取值 | 定义处 |
|---|---|---|
| 模式 | `personal` / `team` | `ServerMode`（serde `rename_all = "lowercase"`）↔ `RuntimeMode`（TS 字面量联合） |
| 角色 | `admin` / `member` | `LocalUserRole` ↔ 迁移的 `DEFAULT 'member'` ↔ TS |
| 状态 | `active` / `disabled` | `LocalUserStatus` ↔ 迁移的 `DEFAULT 'active'` |
| 阶段 | `backlog` / `todo` / `dev` / `review` / `test` / `done` | `StageType::as_str`（`local_project_status.rs:35-43`）↔ `DEFAULT_STATUSES`（`local_project.rs:14-20`）↔ 迁移 `20260917010000` 的 `'test'` ↔ `stageType.ts` 的 `StageType` |
| 提供方 | `feishu` / `lark` / `google` | `OAuthProviderConfig.id` ↔ 路由参数 ↔ `server.json` 的 `oauth` key ↔ i18n key `localAuth.provider*` |

**最容易出错的一条：`StageType` 的 6 个值在 4 个地方各写一遍。** 建议加一条 Rust 测试：
```rust
#[test]
fn 默认状态列与_StageType_一一对应() {
    let stages: Vec<&str> = DEFAULT_STATUSES.iter().map(|(_, _, s)| *s).collect();
    assert_eq!(stages, vec!["backlog", "todo", "dev", "review", "test", "done"]);
}
```
以及一条 TS 测试断言 `Object.keys(STAGE_ORDER)` 等于同样的 6 个值。

### 11.6 函数签名（改了就必须全链路改）

| 签名 | 调用方 |
|---|---|
| `Issues::create(pool, data, creator_user_id)` | `routes/local_projects/issues.rs`、各测试 |
| `IssueComments::create(pool, data, author_id)` | `routes/local_projects/side.rs`、各测试 |
| `projections::handle_workspaces(pool, project_id, owner_user_id)` | `projections.rs` 的 router 闭包、测试 |
| `routes::router(deployment) -> IntoMakeServiceWithConnectInfo<Router, SocketAddr>` | `main.rs:142`、`startup.rs:54` |
| `useKanbanFilters(params) -> { filteredIssues }` | `KanbanContainer.tsx:288` —— **H3 要求签名不变** |
| `makeLocalApiRequest(pathOrUrl, init)` | `api.ts:133`、`remoteApi.ts:117`、`localCollections.ts` 等 9 处 —— **G2 要求签名不变** |

核对命令：`cargo check --workspace && pnpm run check`。

### 11.7 i18n key

新增的每个 key 必须：
1. 在 `packages/web-core/src/i18n/locales/en/<ns>.json` 里存在；
2. 在另外 6 个语言目录的同名文件里存在（`check-i18n.sh` 的 `check_key_consistency`）；
3. 在 `packages/{web-core,local-web,remote-web,ui}/src` 里至少被引用一次（`check-unused-i18n-keys.mjs`）。

核对命令：
```
node scripts/check-unused-i18n-keys.mjs
GITHUB_BASE_REF=main ./scripts/check-i18n.sh
```

---

## 12. 未能核实的假设清单（执行者必须先核实）

> 下面每一条我都**没有**在本仓库里找到确定答案。动手前先核实，核实结果直接写回本文件对应位置。

### 12.1 第三方依赖 API

1. **argon2 0.5 的确切 API**（A5.4）。`Cargo.lock` 里没有 `argon2` / `password-hash`，无法静态核实 `Argon2::new` / `Params::new` / `SaltString::generate` / `PasswordHash::new` 的签名与 `rand_core` 版本兼容性。→ 先 `cargo add argon2@0.5 -p services` 后看 `cargo doc`，以实际为准。
2. **`Params::new(19456, 2, 1, None)` 的参数顺序**是否为 `(m_cost, t_cost, p_cost, output_len)`。→ 以 docs.rs 为准；A5.2 的「PHC 串前缀断言」会兜底抓错。
3. **axum 0.8 的 `axum::serve` 是否接受 `IntoMakeServiceWithConnectInfo`**（C3.4）。→ 以 `cargo check` 为准；若不接受，退回方案：不按 IP 分桶，只按用户名分桶，并在文档里写明「反代后无法按 IP 限速」。
4. **sqlx 0.8 的 `Pool::options()` 是否存在**（F1.1）。→ 若不存在，改成断言常量。
5. **`#[derive(sqlx::Type)]` 对 TEXT 枚举的支持**（A3.3）。→ 不确定就照抄 `local_project_status.rs` 用 `String` + `as_str`/`from_str`。
6. **SQLite `randomblob(16)` 生成的 BLOB 能否被 sqlx 解码成 `Uuid`**（H1.3）。→ 先写一条测试验证；不行就改成应用层回填。
7. **SQLite 外键是否默认开启**（A2.1）。sqlx 的 `SqliteConnectOptions` 默认 `foreign_keys(true)`，但本仓库的 `main_db_options`（`crates/db/src/lib.rs:82-85`）没有显式设置。→ 用 `PRAGMA foreign_keys` 查一次；若是关的，`ON DELETE CASCADE` 不会生效，`revoke_all_for_user` 之外还要手工删会话。

### 12.2 第三方服务端点（任务 E 执行时已部分核实，2026-09-17）

> 核实结论见设计文档 §6.4；代码里的落点是 `crates/services/src/services/local_auth/oauth.rs` 的模块文档。
> **凡未核实的，一律做成配置项，不写死。**

8. ~~**飞书 / Lark 的 authorize / token / userinfo 端点 URL 与版本**~~ → **已核实**。计划 E1.3 表格里的初值**有误**，实际预置值为：
   - 飞书 authorize `https://accounts.feishu.cn/open-apis/authen/v1/authorize`（不是 `open.feishu.cn`）；
   - 飞书 token `https://accounts.feishu.cn/oauth/v3/token`（**v2 已被官方标注弃用**，POST form-urlencoded）；
   - 飞书 userinfo `https://open.feishu.cn/open-apis/authen/v1/user_info`；
   - Lark authorize `https://accounts.larksuite.com/open-apis/authen/v1/authorize`，token `https://open.larksuite.com/open-apis/authen/v2/oauth/token`（POST JSON，**官方文档目前仍是 v2 且无弃用提示**），userinfo `https://open.larksuite.com/open-apis/authen/v1/user_info`。
   - **URL 参数名是 `client_id` 不是 `app_id`**（后台显示为 "App ID"）。
   - 两边版本节奏不同步，因此三个端点 + scopes + pkce 全部做成配置项：`server.json` 的 `oauth.<id>.{authorize_url,token_url,userinfo_url,scopes,pkce}`，或 `VK_OAUTH_<ID>_{AUTHORIZE_URL,TOKEN_URL,USERINFO_URL,SCOPES,PKCE}`。
   - **未核实**：Lark 是否也有 v3 令牌端点 → 用 `token_url` 覆盖项兜底。
9. ~~**飞书 token 响应是否包在 `{"code":0,"data":{...}}` 信封里**~~ → **已核实：不是**。飞书/Lark 的**认证类**接口是扁平结构，`code` 与 `access_token` 同层；`{code,msg,data}` 信封只出现在 `/open-apis/authen/v1/user_info` 这类 open-apis 接口上。所以**不能写一个通用的「剥 data 层」逻辑套所有飞书接口**。实现上换令牌只用一套扁平解析（顶层 `code != 0` 或有 `error` 即失败），拉用户信息才按 `userinfo_envelope` 剥一层。
10. ~~**飞书 / Lark 的 userinfo 字段名**~~ → **已核实：绑定键用 `union_id`**。`open_id` 是应用级的，换应用凭据就变；`user_id` 在管理员删号后可能被复用；email/mobile 官方明文说「未经用户本人实时验证，不建议作为登录凭证」。Google 用 `sub`（官方原文：`Don't use the email field as a unique identifier`）。**未核实**：Lark 的 user_info 响应字段是否与飞书逐字一致 → `subject_field` / `email_field` / `name_field` 已做成 `ResolvedProvider` 上的字段，必要时可加覆盖项。
11. ~~**Google 是否需要 `openid email profile` 三个 scope**~~ → **已核实**：Google 的 `scope` 是必填项，预置 `openid email profile`；对服务端机密客户端不强制 PKCE。飞书支持 PKCE 但不强制（应用一律视为机密客户端，`client_secret` 必带）。**未核实**：Lark 是否支持 PKCE → 预置 `pkce = false`（宁可不带，免得多一个参数被拒），飞书/Google 预置 `true`，都可用 `VK_OAUTH_<ID>_PKCE` 覆盖。**未核实**：飞书/Lark 拿到 `union_id`/`name`/`email` 所需的最小权限点字符串 → 预置为空串（不发 `scope`，用应用后台勾选的权限），需要时用 `VK_OAUTH_<ID>_SCOPES` 配。
12. **各提供方对 `redirect_uri` 的登记要求** → **Google 已核实**：`Redirect URIs must use the HTTPS scheme, not plain HTTP`，且 `Hosts cannot be raw IP addresses`，只有 `localhost`/`127.0.0.1`/`[::1]` 例外——**局域网裸 IP 部署下 Google 登录不可用**。**飞书 / Lark 仍未核实**（最大缺口）：官方文档没有明文规定是否强制 HTTPS 或禁止裸 IP，上线前必须在开发者后台实测能否填入局域网地址。**未核实**：Google「Web application」与「Desktop app」两种客户端类型对 localhost 回调的支持范围（两份官方文档表述有出入）。
    结论不变：**账号密码登录是团队版的主路径**，第三方登录是加分项，登录页按配置动态显示。

**仍需人工完成（上线前）：** 用真实凭据把飞书 / Lark / Google 各走通一次，把实测到的端点 URL、响应字段名、权限点字符串回填到 `oauth.rs` 的预置值里（或直接写进 `server.json`）。

### 12.3 本仓库内未核实的点

13. ~~**`/api/*` 里是否存在用 GET 做写操作的路由**（C1.5）~~ → **已核实（任务 C 执行时，2026-09-17）：存在，共 4 处。**

    扫描方法：`crates/server/src/routes/` 下 64 个 `.rs` 文件，`grep -n "\bget("` 得 119 处字面匹配，剔除 26 处非路由用法（`HashMap::get`、`headers.get`、`Assets::get`、`reqwest::Client::get` 等），剩 93 处真实 `axum::routing::get` 注册；其中 `local_projects/projections.rs` 的一处在 `for table in EMPTY_TABLES`（8 个表名）里展开，故 `/api/*` 下 GET 路由合计约 **100 条**，逐条读 handler 函数体并追进 service / model 层判定副作用。

    | 路由 | 位置 | 副作用 | 严重性 |
    |---|---|---|---|
    | `GET /api/auth/handoff/complete` | `routes/oauth.rs:86`，handler `:156-213`，`finalize_login` `:328-423` | 用 query 里的 `handoff_id`/`app_code` 换 token，`save_credentials` 写凭据文件、`save_config_to_file` 改配置、`tokio::spawn` 拉起 relay 与远端同步 | **高**：经典「OAuth 登录 CSRF」，诱导受害者 GET 一次即可把攻击者账号的凭据写进受害者本机 |
    | `GET /api/workspaces/{id}/git/status` | `routes/workspaces/git.rs:139,373-389` | `ensure_container_exists` → 建 workspace 目录与 git worktree、`UPDATE workspaces` | 中 |
    | `GET /api/workspaces/{id}/integration/editor/path` | `routes/workspaces/integration.rs:141,154-163` | 同上，外加 `container().touch()` | 中 |
    | `GET /api/workspaces/{id}/git/diff/ws` | `routes/workspaces/streams.rs:40-46` | 升级前 `container().touch()` → `UPDATE workspaces SET updated_at` | 低（有防抖，数据不可控） |

    另有两个 WS 端点升级后具备任意命令执行能力（`GET /api/terminal/ws`、`GET /api/ssh-session`），但 `new WebSocket()` 属于子资源请求而非顶层导航，`SameSite=Lax` 不会给它放行 Cookie；C1 的 CSWSH 规则已额外把它们盖住。`GET /api/host/{host_id}/{*tail}` 会把上述攻击面原样代理到配对主机，是放大器而非独立漏洞。

    **结论：`SameSite=Lax` 的前提在本仓库并不成立。** 任务 C 的范围内没有改这 4 条路由（改 `/auth/handoff/complete` 为 POST 会牵连云端登录流程与前端，属于独立任务）。C1 落地后的实际覆盖是：带 Cookie 的**写方法**与 **WebSocket 升级**强制 Origin；带 Cookie 的普通 GET 仍放行缺失 Origin（否则 `<img src="/api/attachments/...">` 这类子资源会被打死）。**遗留风险与后续任务：把 `/api/auth/handoff/complete` 改成 POST，并把 `ensure_container_exists` 从两条 GET 路由里挪走。**
14. **`publish.yml` 的构建路径是否经过 `local-build.sh`**（I6.1）。
15. **`useAppNavigation`（`packages/web-core/src/shared/hooks/useAppNavigation.ts`）暴露了哪个「只改 search 不改 path」的方法**（H5.4）。禁止用 `navigate({to:'.'})` 和 `appNavigation.navigate(...)`，守卫脚本会拒。
16. **`crates/services` 是否可以依赖 `axum`**（E2.1）。当前 `crates/services/Cargo.toml` 没有 axum；本计划已决定把 mock IdP 放 `crates/server`，但若执行者想放 services 需先确认。
17. **`Workspace` 投影里 `owner_user_id` 随请求者变化是否会影响前端**（B5.3）。`USER_WORKSPACES_SHAPE` 在本地模式返回空集合（`localEndpoints.ts`），所以理论上无影响，但要确认 `PROJECT_WORKSPACES_SHAPE` 的消费方（`ProjectProvider.tsx:78-80`）没有按 `owner_user_id` 过滤。
18. **「滑动续期」的确切语义**（设计文档 §5 说明 2）。本计划实现为「只更新 `last_seen_at`，`expires_at` 固定 30 天」。若要真正滑动（每次访问把 `expires_at` 推到 now+30d），需要额外的写库节流，且会让「撤销」更难推理。→ **默认按本计划实现，若产品要求真滑动，在 A4 加 `extend_expiry` 并补测试。**
19. **`tower` 作为 `crates/server` dev-dependency 引入是否顺利**（C2.5）。`Cargo.lock` 里有 `tower 0.5.3`，但它的 `util` feature 是否已启用未知。→ 不顺利就只测抽出的纯异步函数。
20. **`crates/tauri-app` 是否会受会话中间件影响**。它复用 `server::startup::ServerHandle`（`crates/server/src/startup.rs`），默认 personal 模式所以不受影响；但 `pnpm run tauri:dev` 时若环境里残留 `VK_MODE=team` 会登录不了。→ 在文档里提一句。
21. **`dev_assets` 目录下是否需要 `.gitignore` 新增 `machine_token` / `server.json`**。`.gitignore:81` 已忽略整个 `dev_assets`，应无需处理，但要确认 `dev_assets_seed` 里不要误提交这两个文件。
22. **`packages/ui` 的 eslint 是否会对新增的 `LoginPanel.tsx` / `MembersPanel.tsx` / `Skeleton.tsx` 报错**。`pnpm run lint` 会跑 `ui:lint`，但本计划没核实 `packages/ui/.eslintrc*` 的规则。→ 新增组件后立刻跑一次 `pnpm --filter @vibe/ui run lint`。
23. **`KanbanIssuePanel.tsx` 是否被 `remote-web` 使用**。若被使用，H8.3 加 tab 时的「可选 props + 默认不渲染 tab 条」策略必须严格遵守。→ `grep -rn "KanbanIssuePanel" packages/remote-web/src` 确认。

---

## 13. 执行提示

- **每个任务结束后跑一次 `cargo check --workspace`**，不要攒到最后。改 trait（A7）和改函数签名（B2、B3、C3.4）是连锁改动的高发区。
- **不要手改 `shared/types.ts`、`shared/remote-types.ts`、`packages/local-web/src/routeTree.gen.ts`**，它们都是生成物。
- **每次新增 `sqlx::query!` / `query_as!` 后立刻跑 `pnpm run prepare-db`**，否则 CI 的 `prepare-db:check` 会失败，而且失败信息很难定位。
- **所有新 SQL 写在 `crates/db`**（§2.4）。
- **每次新增 i18n key 后立刻跑 `node scripts/check-unused-i18n-keys.mjs` 和 `./scripts/check-i18n.sh`**（§2.8）。
- 安全相关的每一步，**先写攻击样例测试并确认它是红的**，再写实现。红-绿顺序反了就等于没测。
