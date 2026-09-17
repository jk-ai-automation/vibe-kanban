//! 工作区删除申请模型。表结构见
//! `migrations/20260917110000_add_workspace_delete_requests.sql`，
//! 设计见 `docs/superpowers/specs/2026-09-17-workspace-delete-approval-design.md`。
//!
//! 安全约定（每一条都有测试钉着）：
//! - **所有权判据写在 SQL 的 WHERE 里**，不靠调用方先查再比对。撤回一条
//!   别人的申请不是「查出来发现不是自己的就返回错误」，而是那条 UPDATE
//!   根本命中不了行。
//! - **胜者选举只有一个点**：[`WorkspaceDeleteRequests::claim_for_approval`]
//!   的条件 UPDATE。并发批准同一条申请时只有一个拿到 `rows_affected = 1`。
//! - **同一工作区只能有一条待处理申请**由条件唯一索引保证，不是「先查后写」。
//! - 本模块的行**不含**任何本地路径（`container_ref`、worktree 目录），
//!   这样任何 `Json(request)` 都不可能把它们漏到响应里。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use thiserror::Error;
use uuid::Uuid;

use super::db_retry::retry_on_busy;

/// 申请理由 / 驳回理由的长度上限。理由由客户端完全控制，不设上限就等于
/// 让任何登录用户往库里塞任意大的字符串。
pub const MAX_REQUEST_TEXT_LEN: usize = 500;

#[derive(Debug, Error)]
pub enum WorkspaceDeleteRequestError {
    #[error("{0}")]
    Validation(String),
    #[error("申请不存在或已被处理")]
    NotPending,
    #[error(transparent)]
    Database(#[from] sqlx::Error),
}

/// 申请状态。
///
/// `Approved` **只在批准事务内部短暂存在**：批准的最后一步是删工作区，
/// 外键 `ON DELETE CASCADE` 当场把申请行也删了。正常路径下库里不会留下
/// `approved` 行——这是「不留悬空的已批准状态」的机械保证。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkspaceDeleteRequestStatus {
    Pending,
    Approved,
    Rejected,
    Withdrawn,
}

impl WorkspaceDeleteRequestStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Approved => "approved",
            Self::Rejected => "rejected",
            Self::Withdrawn => "withdrawn",
        }
    }
}

/// 一条删除申请，已经把展示需要的工作区名 / 分支 / 申请人用户名 JOIN 进来。
///
/// **字段是白名单**：这里没有 `container_ref`、没有 worktree 路径、
/// 没有申请人的邮箱或密码哈希。加字段前先想清楚它会被谁看到。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorkspaceDeleteRequest {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub workspace_name: Option<String>,
    pub workspace_branch: String,
    pub requested_by_user_id: Option<Uuid>,
    pub requested_by_username: Option<String>,
    pub reason: Option<String>,
    pub status: String,
    pub created_at: DateTime<Utc>,
}

/// 校验并规范化一段用户输入的理由。空白串一律当作「没填」。
pub fn normalize_request_text(
    raw: Option<&str>,
) -> Result<Option<String>, WorkspaceDeleteRequestError> {
    let Some(text) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    // 按**字符数**而不是字节数计：中文理由按字节算会在 167 个字左右就被拒。
    if text.chars().count() > MAX_REQUEST_TEXT_LEN {
        return Err(WorkspaceDeleteRequestError::Validation(format!(
            "理由不得超过 {MAX_REQUEST_TEXT_LEN} 个字符"
        )));
    }
    Ok(Some(text.to_string()))
}

pub struct WorkspaceDeleteRequests;

impl WorkspaceDeleteRequests {
    /// 提交一条删除申请。
    ///
    /// **幂等**：同一工作区已有待处理申请时不新建，直接返回已存在的那条。
    /// 幂等不是靠「先 SELECT 有没有」实现的——那在并发下会插进两条；
    /// 这里靠条件唯一索引 + `ON CONFLICT DO NOTHING`，
    /// 库里永远只有一条 pending。
    pub async fn create_or_get_pending(
        pool: &SqlitePool,
        workspace_id: Uuid,
        requested_by_user_id: Uuid,
        reason: Option<&str>,
    ) -> Result<WorkspaceDeleteRequest, WorkspaceDeleteRequestError> {
        let reason = normalize_request_text(reason)?;
        let id = Uuid::new_v4();
        let now = Utc::now();

        retry_on_busy(|| {
            let reason = reason.clone();
            async move {
                sqlx::query!(
                    r#"INSERT INTO workspace_delete_requests
                           (id, workspace_id, requested_by_user_id, status, reason,
                            created_at, updated_at)
                       VALUES ($1, $2, $3, 'pending', $4, $5, $5)
                       ON CONFLICT DO NOTHING"#,
                    id,
                    workspace_id,
                    requested_by_user_id,
                    reason,
                    now
                )
                .execute(pool)
                .await
            }
        })
        .await?;

        Self::find_pending_by_workspace(pool, workspace_id)
            .await?
            // 插入没命中且此刻也查不到 pending：只可能是在这两步之间被撤回 /
            // 批准了。当作「没有待处理申请」上报，绝不静默返回别的行。
            .ok_or(WorkspaceDeleteRequestError::NotPending)
    }

    pub async fn find_by_id(
        pool: &SqlitePool,
        id: Uuid,
    ) -> Result<Option<WorkspaceDeleteRequest>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceDeleteRequest,
            r#"SELECT r.id                   AS "id!: Uuid",
                      r.workspace_id         AS "workspace_id!: Uuid",
                      w.name                 AS "workspace_name: String",
                      w.branch               AS "workspace_branch!: String",
                      r.requested_by_user_id AS "requested_by_user_id: Uuid",
                      u.username             AS "requested_by_username: String",
                      r.reason               AS "reason: String",
                      r.status               AS "status!: String",
                      r.created_at           AS "created_at!: DateTime<Utc>"
               FROM workspace_delete_requests r
               JOIN workspaces w ON w.id = r.workspace_id
               LEFT JOIN local_users u ON u.id = r.requested_by_user_id
               WHERE r.id = $1"#,
            id
        )
        .fetch_optional(pool)
        .await
    }

    pub async fn find_pending_by_workspace(
        pool: &SqlitePool,
        workspace_id: Uuid,
    ) -> Result<Option<WorkspaceDeleteRequest>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceDeleteRequest,
            r#"SELECT r.id                   AS "id!: Uuid",
                      r.workspace_id         AS "workspace_id!: Uuid",
                      w.name                 AS "workspace_name: String",
                      w.branch               AS "workspace_branch!: String",
                      r.requested_by_user_id AS "requested_by_user_id: Uuid",
                      u.username             AS "requested_by_username: String",
                      r.reason               AS "reason: String",
                      r.status               AS "status!: String",
                      r.created_at           AS "created_at!: DateTime<Utc>"
               FROM workspace_delete_requests r
               JOIN workspaces w ON w.id = r.workspace_id
               LEFT JOIN local_users u ON u.id = r.requested_by_user_id
               WHERE r.workspace_id = $1 AND r.status = 'pending'"#,
            workspace_id
        )
        .fetch_optional(pool)
        .await
    }

    /// 全部待处理申请，最早的在前（先来先审）。
    pub async fn find_all_pending(
        pool: &SqlitePool,
    ) -> Result<Vec<WorkspaceDeleteRequest>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceDeleteRequest,
            r#"SELECT r.id                   AS "id!: Uuid",
                      r.workspace_id         AS "workspace_id!: Uuid",
                      w.name                 AS "workspace_name: String",
                      w.branch               AS "workspace_branch!: String",
                      r.requested_by_user_id AS "requested_by_user_id: Uuid",
                      u.username             AS "requested_by_username: String",
                      r.reason               AS "reason: String",
                      r.status               AS "status!: String",
                      r.created_at           AS "created_at!: DateTime<Utc>"
               FROM workspace_delete_requests r
               JOIN workspaces w ON w.id = r.workspace_id
               LEFT JOIN local_users u ON u.id = r.requested_by_user_id
               WHERE r.status = 'pending'
               ORDER BY r.created_at ASC"#
        )
        .fetch_all(pool)
        .await
    }

    /// 某个用户自己提交的待处理申请。
    ///
    /// **永远不返回别人的行**：过滤条件在 SQL 里，调用方无法跳过。
    pub async fn find_pending_by_requester(
        pool: &SqlitePool,
        requested_by_user_id: Uuid,
    ) -> Result<Vec<WorkspaceDeleteRequest>, sqlx::Error> {
        sqlx::query_as!(
            WorkspaceDeleteRequest,
            r#"SELECT r.id                   AS "id!: Uuid",
                      r.workspace_id         AS "workspace_id!: Uuid",
                      w.name                 AS "workspace_name: String",
                      w.branch               AS "workspace_branch!: String",
                      r.requested_by_user_id AS "requested_by_user_id: Uuid",
                      u.username             AS "requested_by_username: String",
                      r.reason               AS "reason: String",
                      r.status               AS "status!: String",
                      r.created_at           AS "created_at!: DateTime<Utc>"
               FROM workspace_delete_requests r
               JOIN workspaces w ON w.id = r.workspace_id
               LEFT JOIN local_users u ON u.id = r.requested_by_user_id
               WHERE r.status = 'pending' AND r.requested_by_user_id = $1
               ORDER BY r.created_at ASC"#,
            requested_by_user_id
        )
        .fetch_all(pool)
        .await
    }

    /// 撤回自己的待处理申请。返回受影响行数。
    ///
    /// `requested_by_user_id = $2` 写在 SQL 里：撤回别人的申请不是「查出来
    /// 发现不是自己的再报错」，而是这条 UPDATE 命中 0 行，别人的申请
    /// 一个字节都没被碰过。
    pub async fn withdraw(
        pool: &SqlitePool,
        id: Uuid,
        requested_by_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let now = Utc::now();
        retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE workspace_delete_requests
                   SET status = 'withdrawn', updated_at = $3
                   WHERE id = $1 AND requested_by_user_id = $2 AND status = 'pending'"#,
                id,
                requested_by_user_id,
                now
            )
            .execute(pool)
            .await
            .map(|r| r.rows_affected())
        })
        .await
    }

    /// 驳回一条待处理申请。返回受影响行数（0 表示已被撤回 / 已被处理）。
    pub async fn reject(
        pool: &SqlitePool,
        id: Uuid,
        decided_by_user_id: Uuid,
        note: Option<&str>,
    ) -> Result<u64, WorkspaceDeleteRequestError> {
        let note = normalize_request_text(note)?;
        let now = Utc::now();
        let rows = retry_on_busy(|| {
            let note = note.clone();
            async move {
                sqlx::query!(
                    r#"UPDATE workspace_delete_requests
                       SET status = 'rejected', decided_by_user_id = $2,
                           decided_at = $3, decision_note = $4, updated_at = $3
                       WHERE id = $1 AND status = 'pending'"#,
                    id,
                    decided_by_user_id,
                    now,
                    note
                )
                .execute(pool)
                .await
                .map(|r| r.rows_affected())
            }
        })
        .await?;
        Ok(rows)
    }

    /// 批准的**唯一胜者选举点**：`pending` → `approved` 的条件 UPDATE。
    ///
    /// 返回 1 表示本次调用赢得了这条申请，可以去执行删除；返回 0 表示
    /// 申请已被撤回 / 驳回 / 被另一个管理员抢走，**绝不能继续删任何东西**。
    /// 全程没有「先查后写」。
    pub async fn claim_for_approval(
        pool: &SqlitePool,
        id: Uuid,
        decided_by_user_id: Uuid,
    ) -> Result<u64, sqlx::Error> {
        let now = Utc::now();
        retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE workspace_delete_requests
                   SET status = 'approved', decided_by_user_id = $2,
                       decided_at = $3, updated_at = $3
                   WHERE id = $1 AND status = 'pending'"#,
                id,
                decided_by_user_id,
                now
            )
            .execute(pool)
            .await
            .map(|r| r.rows_affected())
        })
        .await
    }

    /// 抢占之后删除失败时的回滚：`approved` → `pending`。
    ///
    /// 不回滚就会留下一条「已批准但工作区还在」的悬空申请——设计规则 3
    /// 明确不要这个状态。
    pub async fn release_claim(pool: &SqlitePool, id: Uuid) -> Result<u64, sqlx::Error> {
        let now = Utc::now();
        retry_on_busy(|| async move {
            sqlx::query!(
                r#"UPDATE workspace_delete_requests
                   SET status = 'pending', decided_by_user_id = NULL,
                       decided_at = NULL, updated_at = $2
                   WHERE id = $1 AND status = 'approved'"#,
                id,
                now
            )
            .execute(pool)
            .await
            .map(|r| r.rows_affected())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{
            local_user::{LocalUser, LocalUserRole, LocalUsers, NewLocalUser},
            workspace::{CreateWorkspace, Workspace},
        },
        test_support::TestDb,
    };

    async fn 建用户(test_db: &TestDb, username: &str) -> LocalUser {
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: username.to_string(),
                display_name: username.to_string(),
                email: None,
                password_hash: Some("$argon2id$x".to_string()),
                role: LocalUserRole::Member,
            },
        )
        .await
        .expect("建用户失败")
    }

    async fn 建工作区(test_db: &TestDb, creator: Uuid, branch: &str) -> Workspace {
        Workspace::create(
            test_db.pool(),
            &CreateWorkspace {
                branch: branch.to_string(),
                name: Some(format!("ws-{branch}")),
            },
            Uuid::new_v4(),
            creator,
        )
        .await
        .expect("建工作区失败")
    }

    async fn 计数(test_db: &TestDb, status: &str) -> i64 {
        sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM workspace_delete_requests WHERE status = ?",
        )
        .bind(status)
        .fetch_one(test_db.pool())
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn 建申请后能按工作区查到待处理() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let req = WorkspaceDeleteRequests::create_or_get_pending(
            test_db.pool(),
            ws.id,
            bob.id,
            Some("没用了"),
        )
        .await
        .expect("建申请失败");

        assert_eq!(req.workspace_id, ws.id);
        assert_eq!(req.requested_by_user_id, Some(bob.id));
        assert_eq!(req.requested_by_username.as_deref(), Some("bob"));
        assert_eq!(req.workspace_branch, "feat-a");
        assert_eq!(req.status, "pending");
        assert_eq!(req.reason.as_deref(), Some("没用了"));

        let found = WorkspaceDeleteRequests::find_pending_by_workspace(test_db.pool(), ws.id)
            .await
            .unwrap()
            .expect("应查到待处理申请");
        assert_eq!(found.id, req.id);
    }

    /// 攻击样例：反复提交想把审批队列刷爆 / 制造两条互相矛盾的申请。
    #[tokio::test]
    async fn 重复提交不产生第二条待处理记录() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let first =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        for _ in 0..5 {
            let again =
                WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                    .await
                    .unwrap();
            assert_eq!(again.id, first.id, "重复提交必须返回同一条");
        }

        assert_eq!(计数(&test_db, "pending").await, 1, "库里只能有一条待处理");
    }

    /// 攻击样例：**另一个人**也来提交同一个工作区的删除申请。
    /// 库里仍然只能有一条，而且是先到的那条（申请人不被顶替）。
    #[tokio::test]
    async fn 别人提交同一工作区也只有一条待处理() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let eve = 建用户(&test_db, "eve").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let first =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        let second =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, eve.id, None)
                .await
                .unwrap();

        assert_eq!(second.id, first.id);
        assert_eq!(
            second.requested_by_user_id,
            Some(bob.id),
            "后来者不得顶替原申请人"
        );
        assert_eq!(计数(&test_db, "pending").await, 1);
    }

    /// 攻击样例：申请人 A 去撤回 B 的申请。
    #[tokio::test]
    async fn 撤回别人的申请不生效() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let eve = 建用户(&test_db, "eve").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();

        let rows = WorkspaceDeleteRequests::withdraw(test_db.pool(), req.id, eve.id)
            .await
            .unwrap();
        assert_eq!(rows, 0, "撤回别人的申请必须命中 0 行");

        let still = WorkspaceDeleteRequests::find_by_id(test_db.pool(), req.id)
            .await
            .unwrap()
            .expect("原申请必须还在");
        assert_eq!(still.status, "pending", "B 的申请不得被动到");
    }

    #[tokio::test]
    async fn 撤回自己的申请后可以再次提交() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let first =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        assert_eq!(
            WorkspaceDeleteRequests::withdraw(test_db.pool(), first.id, bob.id)
                .await
                .unwrap(),
            1
        );
        assert_eq!(计数(&test_db, "pending").await, 0);
        assert_eq!(计数(&test_db, "withdrawn").await, 1);

        let second =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        assert_ne!(second.id, first.id, "撤回后应能提交一条新的");
        assert_eq!(计数(&test_db, "pending").await, 1);
    }

    /// 攻击样例：两个管理员并发批准同一条申请。
    #[tokio::test]
    async fn 抢占只能成功一次() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let amy = 建用户(&test_db, "amy").await;
        let zoe = 建用户(&test_db, "zoe").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();

        let a = WorkspaceDeleteRequests::claim_for_approval(test_db.pool(), req.id, amy.id)
            .await
            .unwrap();
        let b = WorkspaceDeleteRequests::claim_for_approval(test_db.pool(), req.id, zoe.id)
            .await
            .unwrap();
        assert_eq!((a, b), (1, 0), "只有第一次抢占能成功");
    }

    /// 真·并发：同一条申请被多个任务同时抢占，只有一个能赢。
    #[tokio::test]
    async fn 并发抢占只有一个赢家() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let amy = 建用户(&test_db, "amy").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let pool = test_db.pool().clone();
            let request_id = req.id;
            let admin_id = amy.id;
            tasks.push(tokio::spawn(async move {
                WorkspaceDeleteRequests::claim_for_approval(&pool, request_id, admin_id)
                    .await
                    .unwrap()
            }));
        }
        let mut 赢家 = 0;
        for task in tasks {
            赢家 += task.await.unwrap();
        }
        assert_eq!(赢家, 1, "并发抢占只能有一个赢家，实际 {赢家}");
    }

    /// 攻击样例：批准一条已被撤回的申请。
    #[tokio::test]
    async fn 抢占已撤回的申请失败() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let amy = 建用户(&test_db, "amy").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        WorkspaceDeleteRequests::withdraw(test_db.pool(), req.id, bob.id)
            .await
            .unwrap();

        assert_eq!(
            WorkspaceDeleteRequests::claim_for_approval(test_db.pool(), req.id, amy.id)
                .await
                .unwrap(),
            0,
            "已撤回的申请不得被批准"
        );
    }

    #[tokio::test]
    async fn 驳回已撤回或已驳回的申请都失败() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let amy = 建用户(&test_db, "amy").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        assert_eq!(
            WorkspaceDeleteRequests::reject(test_db.pool(), req.id, amy.id, Some("还要用"))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            WorkspaceDeleteRequests::reject(test_db.pool(), req.id, amy.id, None)
                .await
                .unwrap(),
            0,
            "重复驳回必须命中 0 行"
        );
        assert_eq!(
            WorkspaceDeleteRequests::withdraw(test_db.pool(), req.id, bob.id)
                .await
                .unwrap(),
            0,
            "已驳回的申请不能再撤回"
        );
    }

    /// 已驳回 / 已撤回的行不占用「同一工作区只能一条待处理」的名额。
    #[tokio::test]
    async fn 已处理的申请不占用待处理名额() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let amy = 建用户(&test_db, "amy").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;

        for _ in 0..3 {
            let req =
                WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                    .await
                    .unwrap();
            WorkspaceDeleteRequests::reject(test_db.pool(), req.id, amy.id, None)
                .await
                .unwrap();
        }
        assert_eq!(计数(&test_db, "rejected").await, 3);
        assert_eq!(计数(&test_db, "pending").await, 0);
    }

    /// 竞态：申请挂着的时候工作区被管理员直接删掉。
    /// 外键级联必须当场带走申请行，否则队列里会留下一条点不开的僵尸申请。
    #[tokio::test]
    async fn 工作区被删时申请随之消失() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let 别的 = 建工作区(&test_db, bob.id, "feat-b").await;

        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();
        let 别的申请 =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), 别的.id, bob.id, None)
                .await
                .unwrap();

        Workspace::delete(test_db.pool(), ws.id).await.unwrap();

        assert!(
            WorkspaceDeleteRequests::find_by_id(test_db.pool(), req.id)
                .await
                .unwrap()
                .is_none(),
            "外键级联没生效：PRAGMA foreign_keys 可能是 OFF"
        );
        assert!(
            WorkspaceDeleteRequests::find_by_id(test_db.pool(), 别的申请.id)
                .await
                .unwrap()
                .is_some(),
            "不得误伤别的工作区的申请"
        );
    }

    /// 申请人账号被删：申请留着（不然删号就等于绕过审批队列），
    /// 但从此没有人能撤回它——撤回条件要求 `requested_by_user_id` 相等，
    /// NULL 与任何值都不相等。
    #[tokio::test]
    async fn 申请人被删后申请仍在且无人能撤回() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let eve = 建用户(&test_db, "eve").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();

        sqlx::query("DELETE FROM local_users WHERE id = ?")
            .bind(bob.id)
            .execute(test_db.pool())
            .await
            .unwrap();

        let still = WorkspaceDeleteRequests::find_by_id(test_db.pool(), req.id)
            .await
            .unwrap()
            .expect("申请人被删不该带走申请");
        assert_eq!(still.requested_by_user_id, None);
        assert_eq!(still.status, "pending");

        assert_eq!(
            WorkspaceDeleteRequests::withdraw(test_db.pool(), req.id, eve.id)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn 我的申请列表永远只含自己的() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let eve = 建用户(&test_db, "eve").await;
        let ws1 = 建工作区(&test_db, bob.id, "feat-a").await;
        let ws2 = 建工作区(&test_db, eve.id, "feat-b").await;

        WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws1.id, bob.id, None)
            .await
            .unwrap();
        WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws2.id, eve.id, None)
            .await
            .unwrap();

        let 我的 = WorkspaceDeleteRequests::find_pending_by_requester(test_db.pool(), bob.id)
            .await
            .unwrap();
        assert_eq!(我的.len(), 1);
        assert_eq!(我的[0].workspace_id, ws1.id);

        assert_eq!(
            WorkspaceDeleteRequests::find_all_pending(test_db.pool())
                .await
                .unwrap()
                .len(),
            2,
            "管理员队列应能看到两条"
        );
    }

    #[tokio::test]
    async fn 抢占失败可以回滚成待处理() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let amy = 建用户(&test_db, "amy").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();

        WorkspaceDeleteRequests::claim_for_approval(test_db.pool(), req.id, amy.id)
            .await
            .unwrap();
        assert_eq!(
            WorkspaceDeleteRequests::release_claim(test_db.pool(), req.id)
                .await
                .unwrap(),
            1
        );
        assert_eq!(计数(&test_db, "pending").await, 1);
        // 回滚之后必须能被再次抢占。
        assert_eq!(
            WorkspaceDeleteRequests::claim_for_approval(test_db.pool(), req.id, amy.id)
                .await
                .unwrap(),
            1
        );
    }

    #[test]
    fn 理由超长被拒且空白当作没填() {
        assert_eq!(normalize_request_text(None).unwrap(), None);
        assert_eq!(normalize_request_text(Some("   ")).unwrap(), None);
        assert_eq!(
            normalize_request_text(Some("  理由  ")).unwrap().as_deref(),
            Some("理由")
        );
        let 刚好 = "字".repeat(MAX_REQUEST_TEXT_LEN);
        assert!(normalize_request_text(Some(&刚好)).is_ok());
        let 超长 = "字".repeat(MAX_REQUEST_TEXT_LEN + 1);
        assert!(matches!(
            normalize_request_text(Some(&超长)),
            Err(WorkspaceDeleteRequestError::Validation(_))
        ));
    }

    /// 序列化出去的行**不得**含任何本地路径或账号敏感字段。
    #[tokio::test]
    async fn 序列化不泄漏本地路径与敏感字段() {
        let test_db = TestDb::new().await;
        let bob = 建用户(&test_db, "bob").await;
        let ws = 建工作区(&test_db, bob.id, "feat-a").await;
        let req =
            WorkspaceDeleteRequests::create_or_get_pending(test_db.pool(), ws.id, bob.id, None)
                .await
                .unwrap();

        let json = serde_json::to_string(&req).unwrap();
        for 禁词 in [
            "container_ref",
            "worktree",
            "password",
            "token",
            "email",
            "decision_note",
            "decided_by",
        ] {
            assert!(!json.contains(禁词), "响应泄露了 {禁词}：{json}");
        }
    }
}
