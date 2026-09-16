//! 本地账号体系（团队版）的服务端能力：密码哈希、会话令牌、运行时状态。
//!
//! 与 `services::services::auth`（云端 OAuth / JWT）完全无关，不要互相复用；
//! 云端已占用 `AuthContext` 这个名字，本地的运行时叫
//! [`runtime::LocalAuthRuntime`]。

pub mod csrf;
pub mod machine_token;
pub mod password;
pub mod rate_limit;
pub mod runtime;
pub mod token;
