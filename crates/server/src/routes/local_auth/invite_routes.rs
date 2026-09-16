//! 邀请码自助注册（`POST /api/local-auth/invites/accept`），**免鉴权**。
//!
//! 管理员那一头在 `routes/admin/invites.rs`。
//!
//! 安全约定：
//! - 「不存在 / 已过期 / 已使用」三种失败返回**同一个** 400 与同一句文案，
//!   不给攻击者「这个码存不存在」的预言机（枚举防护）。
//! - 新用户的角色只能来自邀请，请求体里没有 `role` 字段可写。
//! - 个人版返回 404：个人版不开放自助注册。
//! - 限速走登录那只限速器，但用**独立的命名空间**，见 [`INVITE_RATE_KEY`]。

use std::{net::SocketAddr, time::Instant};

use axum::{
    extract::{ConnectInfo, State},
    http::{Extensions, HeaderMap, header},
    response::Json as ResponseJson,
};
use chrono::Utc;
use db::models::local_auth::{InviteRedeemError, InviteRegistration, LocalInvites};
use deployment::Deployment;
use serde::Deserialize;
use services::services::{
    local_auth::{
        password::hash_password, rate_limit::client_ip, runtime::LocalAuthRuntime,
        token::hash_invite_code,
    },
    server_settings::ServerMode,
};
use sqlx::SqlitePool;
use ts_rs::TS;
use utils::response::ApiResponse;

use super::{
    LocalAuthUser,
    password_routes::{LoginOutcome, start_session},
};
use crate::{DeploymentImpl, error::ApiError};

/// 邀请注册在限速器里用的「用户名」。
///
/// **不能直接用请求里的用户名**：那样攻击者只要拿着别人的用户名反复
/// 注册失败，就能把那个人的登录桶烧光（限速变成针对受害者的 DoS）。
/// 用一个不可能与真实用户名相撞的固定串（真实用户名只允许 `[a-z0-9._-]`），
/// 让邀请注册自己占一只桶，只与同 IP 的请求互相影响。
const INVITE_RATE_KEY: &str = "\u{2}invite-accept";

/// 三种失败共用的文案。**不得**按原因分化。
const INVALID_CODE_MESSAGE: &str = "邀请码无效或已过期";

/// 注册请求。**刻意没有 `role` 字段**：多传的 JSON 键被 serde 忽略，
/// 角色只能来自邀请码本身。
#[derive(Debug, Clone, Deserialize, TS)]
pub struct AcceptInviteRequest {
    pub code: String,
    pub username: String,
    pub display_name: String,
    pub password: String,
    #[serde(default)]
    pub email: Option<String>,
}

pub(crate) async fn handle_accept_invite(
    pool: &SqlitePool,
    runtime: &LocalAuthRuntime,
    payload: &AcceptInviteRequest,
    user_agent: Option<String>,
    ip: Option<String>,
) -> Result<LoginOutcome, ApiError> {
    // 个人版没有账号体系，这条路由整体不存在。
    if runtime.mode() != ServerMode::Team {
        return Err(ApiError::NotFound);
    }

    let ip_key = ip.as_deref().unwrap_or("");
    // **限速在读库与 Argon2 之前**：否则被限速的请求照样吃掉一次 19 MiB 的哈希。
    if let Err(等待) = runtime
        .login_limiter()
        .check(INVITE_RATE_KEY, ip_key, Instant::now())
    {
        return Err(ApiError::TooManyRequests(format!(
            "尝试过于频繁，请 {} 秒后再试",
            等待.as_secs().max(1)
        )));
    }

    let code_hash = hash_invite_code(payload.code.trim());
    let now = Utc::now();

    // 便宜的预检：码根本不可用就不必为它算一次 Argon2。
    // 这**不是**并发判据——真正的判据是 redeem 里的条件更新。
    if LocalInvites::find_usable_by_code_hash(pool, &code_hash, now)
        .await
        .map_err(ApiError::from)?
        .is_none()
    {
        runtime
            .login_limiter()
            .record_failure(INVITE_RATE_KEY, ip_key, Instant::now());
        return Err(ApiError::BadRequest(INVALID_CODE_MESSAGE.to_string()));
    }

    let password_hash =
        hash_password(&payload.password).map_err(|err| ApiError::BadRequest(err.to_string()))?;

    let user = LocalInvites::redeem(
        pool,
        &code_hash,
        InviteRegistration {
            username: payload.username.clone(),
            display_name: payload.display_name.clone(),
            email: payload.email.clone(),
            password_hash: Some(password_hash),
        },
        now,
    )
    .await
    .map_err(|err| match err {
        InviteRedeemError::InvalidCode => {
            runtime
                .login_limiter()
                .record_failure(INVITE_RATE_KEY, ip_key, Instant::now());
            ApiError::BadRequest(INVALID_CODE_MESSAGE.to_string())
        }
        // 用户名 / 邮箱冲突：邀请码没被消费（redeem 整体回滚），可以换个名字重试。
        // 这不是「猜码失败」，不记限速。
        InviteRedeemError::User(err) => super::password_routes::map_local_user_error(err),
        InviteRedeemError::Database(err) => ApiError::from(err),
    })?;

    // 注册成功等于完成了一次登录，直接下发会话，省掉一次「注册完再登录」。
    runtime.login_limiter().reset(INVITE_RATE_KEY, ip_key);
    start_session(pool, runtime, &user, user_agent, ip).await
}

pub(crate) async fn accept_invite(
    State(deployment): State<DeploymentImpl>,
    extensions: Extensions,
    headers: HeaderMap,
    axum::Json(payload): axum::Json<AcceptInviteRequest>,
) -> Result<(HeaderMap, ResponseJson<ApiResponse<LocalAuthUser>>), ApiError> {
    let runtime = deployment.local_auth();
    let ip = client_ip(
        runtime.trust_proxy(),
        headers
            .get("x-forwarded-for")
            .and_then(|value| value.to_str().ok()),
        extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ConnectInfo(addr)| addr.ip()),
    );
    let user_agent = headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());

    let outcome = handle_accept_invite(
        &deployment.db().pool,
        runtime,
        &payload,
        user_agent,
        Some(ip),
    )
    .await?;
    let response_headers = outcome.headers()?;
    Ok((
        response_headers,
        ResponseJson(ApiResponse::success(outcome.user)),
    ))
}

#[cfg(test)]
mod tests {
    use chrono::Duration;
    use db::{
        models::{
            local_auth::{LocalSessions, NewLocalInvite},
            local_user::{LocalUserRole, LocalUsers, NewLocalUser},
        },
        test_support::TestDb,
    };
    use services::services::{
        local_auth::token::{generate_invite_code, hash_session_token},
        server_settings::ServerSettings,
    };

    use super::*;

    fn runtime(mode: ServerMode) -> LocalAuthRuntime {
        LocalAuthRuntime::new(
            ServerSettings {
                mode,
                ..ServerSettings::default()
            },
            String::new(),
        )
    }

    /// 建一个邀请，返回明文邀请码。
    async fn 建邀请(test_db: &TestDb, role: LocalUserRole, ttl: Duration) -> String {
        let code = generate_invite_code();
        LocalInvites::create(
            test_db.pool(),
            NewLocalInvite {
                code_hash: hash_invite_code(&code),
                role,
                created_by: None,
                expires_at: Utc::now() + ttl,
            },
        )
        .await
        .expect("建邀请失败");
        code
    }

    fn 注册请求(code: &str, username: &str, password: &str) -> AcceptInviteRequest {
        AcceptInviteRequest {
            code: code.to_string(),
            username: username.to_string(),
            display_name: username.to_string(),
            password: password.to_string(),
            email: None,
        }
    }

    #[tokio::test]
    async fn 正确的邀请码可以注册并拿到会话() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;

        let outcome = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect("注册应成功");
        assert_eq!(outcome.user.username, "amy");
        assert_eq!(outcome.user.role, "member");

        let token = outcome
            .session_cookie
            .split(';')
            .next()
            .unwrap()
            .trim_start_matches("vk_session=");
        assert!(
            LocalSessions::find_valid_by_token_hash(
                test_db.pool(),
                &hash_session_token(token),
                Utc::now()
            )
            .await
            .unwrap()
            .is_some(),
            "注册完应当场登录"
        );
    }

    /// **枚举防护**：不存在 / 已过期 / 已使用返回完全相同的状态码与文案。
    #[tokio::test]
    async fn 三种失败的状态码与文案完全一致() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);

        let 不存在 = generate_invite_code();
        let 已过期 = 建邀请(&test_db, LocalUserRole::Member, -Duration::seconds(1)).await;
        let 已使用 = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;
        handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&已使用, "first", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect("首次使用应成功");

        let mut 文案 = Vec::new();
        for (code, 场景) in [(不存在, "不存在"), (已过期, "已过期"), (已使用, "已使用")]
        {
            let err = handle_accept_invite(
                test_db.pool(),
                &rt,
                &注册请求(&code, "later", "hunter2hunter2"),
                None,
                None,
            )
            .await
            .unwrap_err();
            match err {
                ApiError::BadRequest(message) => 文案.push(message),
                other => panic!("{场景} 应是 400，实际 {other:?}"),
            }
        }
        assert_eq!(文案[0], INVALID_CODE_MESSAGE);
        assert!(
            文案.iter().all(|m| m == &文案[0]),
            "三种失败必须一字不差，实际 {文案:?}"
        );
    }

    /// 已使用的邀请码不能重放。
    #[tokio::test]
    async fn 同一个邀请码不能用两次() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;

        handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        let err = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "bob", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect_err("重放必须失败");
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "bob")
                .await
                .unwrap()
                .is_none(),
            "重放不得建出第二个号"
        );
    }

    #[tokio::test]
    async fn 同一邀请码并发注册只有一个成功() {
        let test_db = TestDb::new().await;
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;

        let 跑 = |用户名: &'static str| {
            let pool = test_db.pool().clone();
            let code = code.clone();
            async move {
                let rt = runtime(ServerMode::Team);
                handle_accept_invite(
                    &pool,
                    &rt,
                    &注册请求(&code, 用户名, "hunter2hunter2"),
                    None,
                    None,
                )
                .await
                .map(|o| o.user.username)
            }
        };
        let a = tokio::spawn(跑("racer-a"));
        let b = tokio::spawn(跑("racer-b"));
        let (ra, rb) = (a.await.unwrap(), b.await.unwrap());
        assert_eq!(
            [&ra, &rb].iter().filter(|r| r.is_ok()).count(),
            1,
            "只能有一个成功：{ra:?} / {rb:?}"
        );
    }

    /// 用户名冲突必须 409，而且**邀请码不能被烧掉**——否则谁都能用一个
    /// 已存在的用户名把别人的邀请码作废。
    #[tokio::test]
    async fn 用户名冲突返回_409_且邀请码仍可用() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        LocalUsers::create(
            test_db.pool(),
            NewLocalUser {
                username: "amy".to_string(),
                display_name: "amy".to_string(),
                email: None,
                password_hash: None,
                role: LocalUserRole::Member,
            },
        )
        .await
        .unwrap();
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;

        let err = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "AMY", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect_err("重名应失败");
        assert!(matches!(err, ApiError::Conflict(_)), "实际：{err:?}");

        // 换个名字，同一个码还能用。
        handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "amy2", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect("失败的注册不得消费邀请码");
    }

    /// **角色来自邀请，不来自请求体。** 请求体里多塞一个 `role` 字段会被
    /// serde 直接忽略（DTO 上根本没有这个字段）。
    #[tokio::test]
    async fn 请求体里的_role_被忽略() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;

        let payload: AcceptInviteRequest = serde_json::from_value(serde_json::json!({
            "code": code,
            "username": "amy",
            "display_name": "amy",
            "password": "hunter2hunter2",
            "role": "admin",
            "status": "active",
        }))
        .expect("多余字段应被忽略而不是报错");

        let outcome = handle_accept_invite(test_db.pool(), &rt, &payload, None, None)
            .await
            .unwrap();
        assert_eq!(outcome.user.role, "member", "角色只能来自邀请码");
    }

    #[tokio::test]
    async fn 管理员邀请产生管理员() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Admin, Duration::days(7)).await;
        let outcome = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "boss", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .unwrap();
        assert_eq!(outcome.user.role, "admin");
    }

    #[tokio::test]
    async fn personal_模式下返回_404() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Personal);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;
        let err = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect_err("个人版不开放自助注册");
        assert!(matches!(err, ApiError::NotFound), "实际：{err:?}");
        assert!(
            LocalUsers::find_by_username(test_db.pool(), "amy")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn 弱密码被拒且邀请码仍可用() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;
        let err = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "amy", "1234567"),
            None,
            None,
        )
        .await
        .expect_err("弱密码应 400");
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(
            LocalInvites::find_usable_by_code_hash(
                test_db.pool(),
                &hash_invite_code(&code),
                Utc::now()
            )
            .await
            .unwrap()
            .is_some(),
            "弱密码不得烧掉邀请码"
        );
    }

    #[tokio::test]
    async fn 非法用户名被拒且邀请码仍可用() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;
        let err = handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&code, "ad min", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect_err("非法用户名应 400");
        assert!(matches!(err, ApiError::BadRequest(_)));
        assert!(
            LocalInvites::find_usable_by_code_hash(
                test_db.pool(),
                &hash_invite_code(&code),
                Utc::now()
            )
            .await
            .unwrap()
            .is_some()
        );
    }

    /// 猜码失败要被限速，而且**不能烧掉任何真实用户的登录桶**。
    ///
    /// 受害者用**同一个 IP** 检查：反代默认 `trust_proxy=false`，所有成员到达
    /// 本进程时的对端地址都是反代那一个 IP，所以「同 IP」才是真实场景。
    /// 限速器的组合键是（用户名, IP）——如果这里拿请求里的用户名当键，
    /// 攻击者只要反复用 `amy` 这个名字猜码，就能把 amy 的登录桶烧光，
    /// 限速反过来变成针对 amy 的拒绝服务。
    #[tokio::test]
    async fn 反复猜码会被限速且不影响真实用户登录() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        const 同一个_IP: &str = "10.0.0.7";

        let mut 被限速 = false;
        let mut 次数 = 0;
        for _ in 0..40 {
            次数 += 1;
            let err = handle_accept_invite(
                test_db.pool(),
                &rt,
                &注册请求(&generate_invite_code(), "amy", "hunter2hunter2"),
                None,
                Some(同一个_IP.to_string()),
            )
            .await
            .expect_err("乱猜必失败");
            if matches!(err, ApiError::TooManyRequests(_)) {
                被限速 = true;
                break;
            }
        }
        assert!(被限速, "反复猜码必须被限速（试了 {次数} 次都没挡住）");

        // 关键：受害者 amy 从同一个 IP 登录时，她自己的桶必须还是干净的。
        assert!(
            rt.login_limiter()
                .check("amy", 同一个_IP, Instant::now())
                .is_ok(),
            "邀请注册的限速烧掉了真实用户 amy 的登录桶"
        );
    }

    /// 邀请码前后带空白（从聊天软件里复制粘贴常见）应当照常可用。
    #[tokio::test]
    async fn 邀请码两侧空白被忽略() {
        let test_db = TestDb::new().await;
        let rt = runtime(ServerMode::Team);
        let code = 建邀请(&test_db, LocalUserRole::Member, Duration::days(7)).await;
        handle_accept_invite(
            test_db.pool(),
            &rt,
            &注册请求(&format!("  {code}\n"), "amy", "hunter2hunter2"),
            None,
            None,
        )
        .await
        .expect("两侧空白应被忽略");
    }
}
