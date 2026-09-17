# 工作区删除的管理员审批流程（2026-09-17）

## 0. 一句话

删除工作区**只有管理员做得了**，这条线画在服务端；非管理员点删除变成「提交删除申请」，
管理员批准的那一刻就把工作区删掉，不留「已批准但没删」的中间态。

个人版行为**零变化**：迁移写入的本机用户（`DEFAULT_USER_ID`）本来就是 admin，
同一道守卫对它恒为真。

## 1. 为什么要做

核实过的现状（`main` @ `fae19de8`）：

- `crates/server/src/routes/workspaces/core.rs::delete_workspace` **没有任何鉴权**。
  `/api/workspaces/{id}` 的 DELETE 只被 `require_local_session` 包着——任何登录用户，
  包括 member，都能删掉别人的工作区。
- 前端 `IssueWorkspacesSection.tsx` 用 `workspace.isOwnedByCurrentUser` 把删除菜单项藏起来，
  但那只是 UI：`curl -X DELETE` 照删不误。
- `workspaces.created_by_user_id`（迁移 `20260917010000`）与投影层的 `owner_user_id`
  已经是真实创建者，所以前端的「是不是我的」判断本身是对的，缺的只是服务端那一刀。

## 2. 数据模型

新表 `workspace_delete_requests`（迁移 `20260917110000_add_workspace_delete_requests.sql`）：

| 列 | 类型 | 说明 |
| --- | --- | --- |
| `id` | BLOB PK | **必须是第 0 列**——`crates/services/src/services/events.rs` 的 preupdate 钩子用 `get_old_column_value(0)` 取主键。本表暂不进钩子白名单，但列序照规矩排，免得以后加进去时踩坑 |
| `workspace_id` | BLOB NOT NULL | `REFERENCES workspaces(id) ON DELETE CASCADE` |
| `requested_by_user_id` | BLOB | `REFERENCES local_users(id) ON DELETE SET NULL` |
| `status` | TEXT NOT NULL | `pending` / `rejected` / `withdrawn`（`approved` 不会落盘，见 §4） |
| `reason` | TEXT | 申请理由，≤ 500 字符（服务端截断前先校验并 400） |
| `decided_by_user_id` | BLOB | `REFERENCES local_users(id) ON DELETE SET NULL` |
| `decided_at` | TEXT | RFC3339 |
| `decision_note` | TEXT | 驳回理由，≤ 500 字符 |
| `created_at` / `updated_at` | TEXT NOT NULL | RFC3339，与全库一致 |

### 2.1 「同一工作区只能有一条待处理申请」

用**条件唯一索引**，不是「先查后写」：

```sql
CREATE UNIQUE INDEX idx_workspace_delete_requests_one_pending
    ON workspace_delete_requests (workspace_id)
    WHERE status = 'pending';
```

重复提交由数据库拒绝（`SQLITE_CONSTRAINT_UNIQUE`），应用层把它映射成
「返回已存在的那条 pending」而不是 409——前端的语义是「点了申请删除」，
已经有一条就直接把那条显示出来，用户体验上是幂等的。
**关键点是：并发两个请求打进来，库里也只会有一条。**

### 2.2 工作区被删时申请怎么办：`ON DELETE CASCADE`

理由：申请是「请求删除某个工作区」，工作区没了，申请就失去了指向。用 `SET NULL`
会留下一条 `workspace_id IS NULL` 的僵尸 pending，管理员在队列里看到一条点不开的申请。
级联删除天然处理了竞态 §5.3（申请期间管理员直接删了工作区 → 申请随之消失 →
再去批准得到 404「申请不存在」，不 panic、更不会误删别的东西）。

审计留痕不靠这张表：批准/驳回都走 `tracing::info!` + `track_if_analytics_allowed`。
这是**刻意的取舍**——把审计放进业务表会逼着表容忍「指向不存在工作区」的行。

### 2.3 申请人 / 审批人被删除：`ON DELETE SET NULL`

与同库的 `workspaces.created_by_user_id`、`local_invites.created_by` 一致。
账号注销不该连带抹掉一条待处理申请（那等于「删号即绕过审批队列」）。
`requested_by_user_id IS NULL` 的 pending 申请：管理员照样能批/驳，
但**没有人能撤回它**（撤回要求 `requested_by_user_id = 当前用户`，NULL 永不相等）。

## 3. 接口

### 3.1 直接删除（改现有接口）

`DELETE /api/workspaces/{id}` —— 在 handler 第一行加 `require_admin(actor)?`，**无条件**。

为什么不按模式分支（`if mode == Team`）：个人版与团队版本机进程走的都是
`SessionGate::PersonalBypass`，注入的是迁移里那条 `role='admin'` 的本机用户，
守卫对它恒为真。少一个分支就少一处「团队版忘了包住」的可能。

配套加固（否则上面这句「恒为真」不成立）：`admin/users.rs::handle_update_user`
**拒绝修改本机固定用户（`DEFAULT_USER_ID`）的角色**。现状后端没拦，
`确保仍有可登录管理员` 也拦不住（本机用户没有凭据，不算「可登录管理员」），
只有前端 `members.ts::isFixedLocalUser` 拦——把它降成 member 会让 MCP / 本机进程
再也删不掉工作区，个人版也会跟着退化。这条加固让守卫的前提变成结构性的。

### 3.2 申请侧（非管理员，挂在受保护组，不在 `/api/admin` 下）

| 方法 | 路径 | 谁能调 | 行为 |
| --- | --- | --- | --- |
| POST | `/api/workspace-delete-requests` | 任何登录用户，**admin 除外** | 建申请。body `{ workspace_id, reason? }`。admin 调 → 400「管理员可直接删除」 |
| GET | `/api/workspace-delete-requests` | 任何登录用户 | **只返回自己提交的** pending 申请（admin 调也一样，不是审批队列） |
| DELETE | `/api/workspace-delete-requests/{id}` | 仅申请人 | 撤回。非本人 → 404 |

撤回别人的申请返回 **404 而不是 403**：403 会确认「这个 id 存在」，
让 member 能枚举别人的申请 id。这里 id 本身是要保密的，与 `/api/admin` 的
「路径不是秘密」不同。

### 3.3 审批侧（`/api/admin/*`，自动被 `require_admin_middleware` 覆盖）

| 方法 | 路径 | 行为 |
| --- | --- | --- |
| GET | `/api/admin/workspace-delete-requests` | 全部 pending，带工作区名/分支、申请人用户名 |
| POST | `/api/admin/workspace-delete-requests/{id}/approve` | **批准即删除** |
| POST | `/api/admin/workspace-delete-requests/{id}/reject` | 驳回，body `{ note? }` |

`/api/admin` 组已经在 `routes/admin/mod.rs` 里被 `.layer(require_admin_middleware)` 整组包住，
新增的三条自动被覆盖；三个纯逻辑函数额外各自再调一次 `require_admin(actor)?`
（那里有条测试逐个函数扫源码钉着）。`admin_endpoints()` 契约测试也要同步加这三条。

CSRF：写方法由 `require_local_session` 里的 `csrf_check` 统一处理，本模块不额外做事。

### 3.4 响应字段（不泄漏）

```
WorkspaceDeleteRequestInfo {
  id, workspace_id, workspace_name, workspace_branch,
  requested_by_user_id, requested_by_username,
  reason, status, created_at
}
```

**不含** `container_ref`、worktree 路径、申请人邮箱、`password_hash`。
有一条测试把序列化结果拿去搜这些字段名。

## 4. 状态机

```
            POST /workspace-delete-requests
                        │
                        ▼
                    ┌────────┐
        withdraw ◄──┤pending ├──► reject ──► rejected（留档）
       （仅申请人）   └───┬────┘
      withdrawn           │ approve（仅管理员）
      （留档）             ▼
                  条件 UPDATE 抢占 → 执行删除 → 工作区行消失
                                              → 级联带走本申请行
```

`approved` 这个状态**只在一个事务内部短暂存在**，正常路径下从不落盘：
批准的最后一步是删工作区，外键级联当场把申请行也删了。这正是规则 3
「不留悬空的已批准状态」的机械保证——不是靠调用方记得清理。

### 4.1 批准的具体步骤（顺序是安全性的一部分）

1. 取申请 → 取工作区（任一不存在 → 404）。工作区取不到就**绝不**继续，
   不拿「同 id 的别的东西」去走删除路径。
2. **抢占**：`UPDATE ... SET status='approved', decided_by=?, decided_at=?
   WHERE id=? AND status='pending'`，看 `rows_affected`。
   这是唯一的胜者选举点，全程不「先查后写」。`rows_affected = 0`
   → 已被撤回 / 已被驳回 / 已被另一个管理员批准 → 409，**不删任何东西**。
3. 执行删除，走的是与「管理员直接删」**同一个** `perform_workspace_deletion`
   （前置检查「有进程在跑」→ 停 dev server → `prepare_deletion_context`
   → `delete_record`）。工作区行一删，级联带走申请行。
4. 删除失败（有进程在跑 / IO / 数据库异常）→ 把申请**条件回滚**成 `pending`
   （`WHERE id=? AND status='approved'`），避免留下悬空状态。
   前置检查因此排在抢占**之后**：与其在两处各写一份「能不能删」的判据，
   不如让唯一的那份留在共用的执行段里，失败由回滚兜住。

## 5. 竞态处理

| 竞态 | 处理 |
| --- | --- |
| 5.1 重复提交申请 | 条件唯一索引挡住；应用层把唯一冲突翻译成「返回已存在的那条」。**库里永远只有一条 pending** |
| 5.2 两个管理员并发批准 | §4.1 第 3 步的条件 UPDATE：只有一个拿到 `rows_affected = 1`，另一个 409。真正的删除只发生一次 |
| 5.3 申请期间工作区已被管理员直接删掉 | 外键 `ON DELETE CASCADE` 当场删掉申请行 → 批准取不到申请 → 404。不 panic，也不会去删「同 id 的别的东西」 |
| 5.4 批准时申请已被申请人撤回 | 撤回把 `status` 改成 `withdrawn` → 条件 UPDATE 命中 0 行 → 409 |
| 5.5 申请人撤回别人的申请 | `DELETE ... WHERE id=? AND requested_by_user_id=? AND status='pending'`，`rows_affected=0` → 404。**条件写在 SQL 里**，不靠调用方比对 |
| 5.6 撤回一条已被批准正在删的申请 | 抢占已经把 `status` 改成 `approved`，撤回的条件 UPDATE 命中 0 行 → 404 |

## 6. 权限矩阵

| 操作 | 个人版本机用户 | 团队版 admin | 团队版 member（含工作区创建者） | 未登录 |
| --- | --- | --- | --- | --- |
| `DELETE /api/workspaces/{id}` | ✅ 直接删 | ✅ 直接删 | ❌ 403，**工作区仍在** | 401 |
| 建删除申请 | ❌ 400（它是 admin，直接删就行） | ❌ 400 | ✅ | 401 |
| 看自己的申请 | ✅（永远为空） | ✅（永远为空） | ✅ | 401 |
| 撤回申请 | —— | 仅自己的（不存在） | ✅ 仅自己的 | 401 |
| 看待处理队列 | ✅（永远为空） | ✅ | ❌ 403 | 401 |
| 批准 / 驳回 | ✅（队列为空） | ✅ | ❌ 403 | 401 |

## 7. 个人版与团队版的差异

服务端：**没有任何 `if mode == Personal` 分支**。个人版只有一个用户且是 admin，
所有守卫对它恒为真，删除路径逐字不变（同一个 `perform_workspace_deletion`）。
申请类接口在个人版存在但永远是空的：本机用户是 admin，建申请被 400 挡掉。

前端：门禁用**新增的 `isLocalTeamMode()`**（本地数据源 **且** 需要登录），
不是 `!isLocalPersonalMode()`。两者不是互补关系——云端构建的个人模式
（数据源 `remote`）满足那个取反，但它根本连不上本机的
`/api/workspace-delete-requests`，用取反会让云端界面上多出一个永远报错的区块。
`isLocalTeamMode()` 为假时查询 `enabled: false`（一个请求都不发）、
`resolveDeleteAffordance` 返回 `undefined`，删除菜单项文案与行为与今天逐字一致。

## 8. 前端放置

1. **工作区卡片（`IssueWorkspaceCard`）**——主入口。
   - 非管理员：删除菜单项文案变成「申请删除」。
   - 有待处理申请：卡片上出现「待审批删除」徽标；管理员在菜单里就地「批准删除 / 驳回」，
     申请人在菜单里「撤回删除申请」。
   - 理由：申请是针对**某个具体工作区**发起的，决策所需的上下文（分支、改动量、PR 状态）
     全在这张卡上；跳去别的页面批准等于让管理员盲批。
2. **成员管理页新增「删除申请」区块**——兜底队列。
   - 理由：申请可能来自任何 issue 下的任何工作区，管理员不该靠翻看板去发现它。
     成员管理页已经是「团队治理」的落点（成员、邀请码），删除审批是同一类事务，
     而且那页本来就只有 admin 能进，权限语义天然对上。

文案全部走 i18n，7 个语言文件同步；key 用**字面量映射表**，不用模板字面量拼接
（`scripts/check-unused-i18n-keys.mjs` 扫不到模板拼出来的 key，会误报未使用）。

看板（`KanbanContainer`）上的工作区卡片本来就没有删除入口（不传 `onDelete`），
本期不给它加——那会把「在看板上误删一个工作区」变成一次点击的距离。

## 9. 不做什么

- 不做「已批准待执行」的队列（规则 3 明确不要）。
- 不做邮件/推送通知。
- 不做申请历史页（rejected / withdrawn 只留在库里，接口不暴露列表）。
- 不动 `crates/remote`（本 fork 缺私有依赖，`pnpm run check` 也因此不跑）。
