//! SQLite 写锁争抢时的通用重试。
//!
//! 个人版的库跑在 rollback-journal 模式下，多个连接同时写会拿到
//! BUSY/LOCKED；`sqlite3_busy_timeout` 只能缓解（它不覆盖 upgrade 死锁等情形），
//! 因此应用层仍要自己重试。所有写路径（单条更新、批量更新、建需求）共用这里的实现。

use std::{future::Future, time::Duration};

/// 重试次数上限。
pub const BUSY_RETRY: u32 = 8;

/// 判断一个数据库错误是否值得重试：SQLite 争抢写锁时返回的 BUSY/LOCKED 系列错误码。
pub fn is_retryable_db_error(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };
    // SQLITE_BUSY = 5, SQLITE_LOCKED = 6, SQLITE_BUSY_RECOVERY = 261, SQLITE_BUSY_SNAPSHOT = 517
    matches!(
        db_err.code().as_deref(),
        Some("5") | Some("6") | Some("261") | Some("517")
    )
}

/// 是否是唯一约束/主键冲突。客户端自带 id 重复提交时会撞上，
/// 这属于 409 而不是 500。
pub fn is_unique_violation(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db_err) = err else {
        return false;
    };
    // SQLITE_CONSTRAINT_UNIQUE = 2067, SQLITE_CONSTRAINT_PRIMARYKEY = 1555
    matches!(db_err.code().as_deref(), Some("2067") | Some("1555"))
}

/// 可重试的错误类型。模型层的错误枚举实现它之后就能套用 [`retry_on_busy`]。
pub trait RetryableDbError {
    fn is_busy(&self) -> bool;
}

impl RetryableDbError for sqlx::Error {
    fn is_busy(&self) -> bool {
        is_retryable_db_error(self)
    }
}

/// 反复执行 `operation`，直到成功、错误不可重试、或用尽 [`BUSY_RETRY`] 次机会。
///
/// `operation` 每次都要重新开事务：重试发生在事务回滚之后，不能复用旧事务。
pub async fn retry_on_busy<T, E, F, Fut>(mut operation: F) -> Result<T, E>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T, E>>,
    E: RetryableDbError,
{
    let mut attempt: u32 = 0;
    loop {
        match operation().await {
            Ok(value) => return Ok(value),
            Err(err) if err.is_busy() && attempt + 1 < BUSY_RETRY => {
                attempt += 1;
                // 线性退避：并发写多为毫秒级冲突，指数退避反而拖慢整体。
                tokio::time::sleep(Duration::from_millis(5 * attempt as u64)).await;
            }
            Err(err) => return Err(err),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::{BUSY_RETRY, RetryableDbError, retry_on_busy};

    #[derive(Debug, PartialEq)]
    enum 假错误 {
        繁忙,
        致命,
    }

    impl RetryableDbError for 假错误 {
        fn is_busy(&self) -> bool {
            matches!(self, 假错误::繁忙)
        }
    }

    #[tokio::test]
    async fn 繁忙错误会重试直到成功() {
        let 次数 = Cell::new(0u32);
        let result: Result<u32, 假错误> = retry_on_busy(|| async {
            次数.set(次数.get() + 1);
            if 次数.get() < 3 {
                Err(假错误::繁忙)
            } else {
                Ok(次数.get())
            }
        })
        .await;

        assert_eq!(result.unwrap(), 3);
        assert_eq!(次数.get(), 3);
    }

    #[tokio::test]
    async fn 致命错误立即返回不重试() {
        let 次数 = Cell::new(0u32);
        let result: Result<u32, 假错误> = retry_on_busy(|| async {
            次数.set(次数.get() + 1);
            Err(假错误::致命)
        })
        .await;

        assert_eq!(result.unwrap_err(), 假错误::致命);
        assert_eq!(次数.get(), 1, "不可重试的错误不得重试");
    }

    #[tokio::test]
    async fn 一直繁忙时用尽重试次数后放弃() {
        let 次数 = Cell::new(0u32);
        let result: Result<u32, 假错误> = retry_on_busy(|| async {
            次数.set(次数.get() + 1);
            Err(假错误::繁忙)
        })
        .await;

        assert_eq!(result.unwrap_err(), 假错误::繁忙);
        assert_eq!(次数.get(), BUSY_RETRY, "重试次数必须有上限");
    }
}
