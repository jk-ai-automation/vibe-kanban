//! Argon2id 密码哈希。
//!
//! 只用成熟库（`argon2` / `password-hash`），不自造任何密码学原语。
//! 参数取 OWASP 推荐档：m=19456 KiB、t=2、p=1。

use std::sync::OnceLock;

use argon2::{
    Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version,
    password_hash::{SaltString, rand_core::OsRng},
};
use thiserror::Error;

/// 密码长度下限（字节）。
pub const MIN_PASSWORD_LEN: usize = 8;

/// 密码长度上限（字节）。
///
/// 必须有上限：每次 Argon2 调用都要吃 19 MiB 内存，而 `argon2` crate 自己的
/// `MAX_PWD_LEN` 是 4 GiB（等于没有上限），不在这里挡住就是现成的 DoS 向量。
pub const MAX_PASSWORD_LEN: usize = 1024;

/// 生成新哈希时用的内存代价（KiB）。
const HASH_M_COST: u32 = 19456;
const HASH_T_COST: u32 = 2;
const HASH_P_COST: u32 = 1;

/// 校验时允许的参数上限。PHC 串来自数据库，正常情况下是我们自己写进去的，
/// 但一旦库被改坏（或被注入），`m=4294967295` 会让 argon2 去申请 4 TiB 内存。
/// 超出白名单一律判为「这不是我们写的哈希」。
const MAX_VERIFY_M_COST: u32 = 1 << 16; // 64 MiB
const MAX_VERIFY_T_COST: u32 = 10;
const MAX_VERIFY_P_COST: u32 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum PasswordError {
    #[error("密码至少需要 {MIN_PASSWORD_LEN} 个字节")]
    TooShort,
    #[error("密码不能超过 {MAX_PASSWORD_LEN} 个字节")]
    TooLong,
    #[error("密码哈希失败")]
    HashFailed,
}

fn hasher() -> Argon2<'static> {
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(HASH_M_COST, HASH_T_COST, HASH_P_COST, None).expect("固定参数必须合法"),
    )
}

/// 计算一个密码的 Argon2id PHC 串。
///
/// **不做 trim、不做 Unicode 归一化**：密码是字节串，任何折叠都会凭空缩小口令空间。
/// 唯一的例外是「全是空白」按空密码处理，因为那几乎一定是输入框漏填。
pub fn hash_password(plain: &str) -> Result<String, PasswordError> {
    check_length(plain)?;
    if plain.trim().is_empty() {
        return Err(PasswordError::TooShort);
    }

    let salt = SaltString::generate(&mut OsRng);
    hasher()
        .hash_password(plain.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordError::HashFailed)
}

/// 校验密码。
///
/// 返回 `bool` 而不是 `Result` 是刻意的：调用方无法把 `Err` 误当成「通过」。
/// 任何解析失败、参数越界、算法不符都返回 `false`，绝不 panic。
pub fn verify_password(plain: &str, phc: &str) -> bool {
    // 先挡长度：不能让攻击者用超长请求体驱动 Argon2。
    if plain.len() > MAX_PASSWORD_LEN {
        return false;
    }
    let Ok(parsed) = PasswordHash::new(phc) else {
        return false;
    };
    let Ok(params) = Params::try_from(&parsed) else {
        return false;
    };
    if params.m_cost() > MAX_VERIFY_M_COST
        || params.t_cost() > MAX_VERIFY_T_COST
        || params.p_cost() > MAX_VERIFY_P_COST
    {
        return false;
    }
    hasher().verify_password(plain.as_bytes(), &parsed).is_ok()
}

/// 一个谁也不知道原文的合法 PHC 串，专门用来在「用户不存在」时照样跑一遍
/// Argon2，让成功与失败两条路径耗时一致（防用户枚举）。
///
/// 原文是进程启动后随机生成的 32 字节，既不写在源码里也不落盘，
/// 所以它永远不可能对任何输入返回 `true`。
pub fn dummy_password_hash() -> &'static str {
    static DUMMY: OnceLock<String> = OnceLock::new();
    DUMMY.get_or_init(|| {
        use rand::RngCore;
        let mut secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut secret);
        let salt = SaltString::generate(&mut OsRng);
        hasher()
            .hash_password(&secret, &salt)
            .expect("固定参数下哈希不会失败")
            .to_string()
    })
}

fn check_length(plain: &str) -> Result<(), PasswordError> {
    if plain.len() < MIN_PASSWORD_LEN {
        return Err(PasswordError::TooShort);
    }
    if plain.len() > MAX_PASSWORD_LEN {
        return Err(PasswordError::TooLong);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 同一密码两次哈希结果不同() {
        let a = hash_password("correct horse battery").unwrap();
        let b = hash_password("correct horse battery").unwrap();
        assert_ne!(a, b, "盐必须随机");
        assert!(verify_password("correct horse battery", &a));
        assert!(verify_password("correct horse battery", &b));
    }

    #[test]
    fn 正确密码校验通过而错误密码失败() {
        let phc = hash_password("hunter2hunter2").unwrap();
        assert!(verify_password("hunter2hunter2", &phc));
        for wrong in [
            "hunter2hunter",
            "hunter2hunter2 ",
            " hunter2hunter2",
            "HUNTER2HUNTER2",
            "hunter2hunter3",
            "",
        ] {
            assert!(!verify_password(wrong, &phc), "{wrong:?} 不应通过");
        }
    }

    #[test]
    fn 空密码与过短密码被拒绝() {
        assert!(matches!(hash_password(""), Err(PasswordError::TooShort)));
        assert!(matches!(
            hash_password("       "),
            Err(PasswordError::TooShort)
        ));
        assert!(matches!(
            hash_password(&"a".repeat(MIN_PASSWORD_LEN - 1)),
            Err(PasswordError::TooShort)
        ));
        assert!(hash_password(&"a".repeat(MIN_PASSWORD_LEN)).is_ok());
    }

    /// 上限是 DoS 防护：Argon2 的每次调用都要吃 19 MiB 内存，
    /// 而 argon2 crate 本身的 MAX_PWD_LEN 是 4 GiB，等于没有上限。
    #[test]
    fn 超长密码被拒绝而不是拿去做哈希() {
        assert!(matches!(
            hash_password(&"a".repeat(MAX_PASSWORD_LEN + 1)),
            Err(PasswordError::TooLong)
        ));
        assert!(hash_password(&"a".repeat(MAX_PASSWORD_LEN)).is_ok());

        // 校验路径同样要挡：否则攻击者可以用 1 MB 的登录请求体压垮服务。
        let phc = hash_password("hunter2hunter2").unwrap();
        assert!(!verify_password(&"a".repeat(MAX_PASSWORD_LEN + 1), &phc));
        assert!(!verify_password(&"a".repeat(1024 * 1024), &phc));
    }

    /// 长度按**字节**算。多字节字符不能绕过上限，也不能让一个 4 字符的
    /// emoji 密码被当成过短。
    #[test]
    fn 长度按字节计算() {
        // 每个字符 4 字节，256 个 = 1024 字节，正好卡在上限。
        let 刚好 = "🙂".repeat(MAX_PASSWORD_LEN / 4);
        assert_eq!(刚好.len(), MAX_PASSWORD_LEN);
        assert!(hash_password(&刚好).is_ok());
        assert!(matches!(
            hash_password(&"🙂".repeat(MAX_PASSWORD_LEN / 4 + 1)),
            Err(PasswordError::TooLong)
        ));

        // 2 个 emoji = 8 字节，达到下限。
        assert!(hash_password("🙂🙂").is_ok());
        assert!(matches!(hash_password("🙂"), Err(PasswordError::TooShort)));
    }

    /// 刻意**不做** Unicode 归一化：NFC 的 "é"(U+00E9) 与 NFD 的 "é"(e+U+0301)
    /// 是不同的字节串，因此是不同的密码。归一化会把两个不同的输入折叠成同一个
    /// 密码，等于凭空缩小口令空间；这里用测试把「不归一化」这个决定钉死，
    /// 免得以后有人「顺手加个 nfc()」。
    #[test]
    fn 不做_unicode_归一化() {
        let nfc = "café1234"; // é = U+00E9
        let nfd = "cafe\u{301}1234"; // e + U+0301
        assert_ne!(nfc.as_bytes(), nfd.as_bytes());

        let phc = hash_password(nfc).unwrap();
        assert!(verify_password(nfc, &phc));
        assert!(
            !verify_password(nfd, &phc),
            "不同字节串必须是不同密码；一旦这条失败说明有人加了归一化"
        );
    }

    #[test]
    fn 密码里的空白与控制字符原样保留() {
        // 只有「全是空白」才算空密码；密码本身可以含空格。
        let phc = hash_password("  a b c  ").unwrap();
        assert!(verify_password("  a b c  ", &phc));
        assert!(!verify_password("a b c", &phc), "不得对密码做 trim");

        let phc = hash_password("pass\0word").unwrap();
        assert!(verify_password("pass\0word", &phc));
        assert!(
            !verify_password("pass", &phc),
            "不得在 NUL 处截断（C 字符串陷阱）"
        );
    }

    #[test]
    fn 畸形哈希串校验返回_false_而不是_panic() {
        for bad in [
            "",
            "   ",
            "not-a-phc-string",
            "$argon2id$",
            "$argon2id$v=19$m=19456,t=2,p=1$",
            "$argon2id$v=19$m=19456,t=2,p=1$c2FsdA", // 缺哈希段
            "$bcrypt$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA",
            "$argon2id$v=99999$m=19456,t=2,p=1$c2FsdA$aGFzaA",
            "$$$$$$",
            "\0",
        ] {
            assert!(
                !verify_password("hunter2hunter2", bad),
                "{bad:?} 必须返回 false"
            );
        }
    }

    /// 改掉 PHC 串里的任何一段（盐、摘要、参数）都必须让校验失败。
    /// 如果只改参数就能通过，说明实现没有真的重算摘要。
    #[test]
    fn 哈希参数或摘要被篡改后校验失败() {
        let phc = hash_password("hunter2hunter2").unwrap();

        // 把 m_cost 调低：摘要对不上，必须失败（而不是「按新参数重算后通过」）。
        let 改参数 = phc.replace("m=19456", "m=8");
        assert_ne!(改参数, phc);
        assert!(!verify_password("hunter2hunter2", &改参数));

        // 改 t_cost。
        let 改轮数 = phc.replace("t=2", "t=1");
        assert_ne!(改轮数, phc);
        assert!(!verify_password("hunter2hunter2", &改轮数));

        // 改最后一个字符（摘要段）。
        let mut 改摘要: Vec<char> = phc.chars().collect();
        let last = 改摘要.len() - 1;
        改摘要[last] = if 改摘要[last] == 'A' { 'B' } else { 'A' };
        let 改摘要: String = 改摘要.into_iter().collect();
        assert_ne!(改摘要, phc);
        assert!(!verify_password("hunter2hunter2", &改摘要));

        // 改算法标识（argon2id → argon2i）。
        let 改算法 = phc.replace("$argon2id$", "$argon2i$");
        assert!(!verify_password("hunter2hunter2", &改算法));
    }

    /// PHC 串里含天文数字的 m_cost 时，校验必须直接拒绝而不是照着它去分配内存。
    #[test]
    fn 哈希串里的离谱参数不会被拿去分配内存() {
        let phc = hash_password("hunter2hunter2").unwrap();
        let 炸弹 = phc.replace("m=19456", "m=4294967295");
        assert!(
            !verify_password("hunter2hunter2", &炸弹),
            "超出白名单的参数必须直接拒绝"
        );
    }

    #[test]
    fn 参数是_owasp_档位() {
        let phc = hash_password("hunter2hunter2").unwrap();
        assert!(
            phc.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
            "PHC 串前缀不符合预期：{phc}"
        );
    }

    #[test]
    fn 哈希串里不含明文密码() {
        let phc = hash_password("SuperSecret123").unwrap();
        assert!(!phc.contains("SuperSecret123"));
    }

    /// 防用户枚举用的假哈希必须是一个**合法**的 PHC 串（否则 verify 会在
    /// 解析阶段就返回，根本跑不到 Argon2，耗时差异依然存在），
    /// 同时对任何输入都返回 false。
    #[test]
    fn 假哈希是合法_phc_且永不通过() {
        let dummy = dummy_password_hash();
        assert!(dummy.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert!(PasswordHash::new(dummy).is_ok());
        for guess in ["", "hunter2hunter2", "admin", &"a".repeat(64)] {
            assert!(!verify_password(guess, dummy));
        }
        // 同一进程内稳定，不会每次调用都重算。
        assert_eq!(dummy, dummy_password_hash());
    }

    /// 登录失败的两条路径（用户不存在 / 密码错）耗时必须同量级。
    /// 这里不追求纳秒级恒定，只要求「假哈希路径确实跑了一遍 Argon2」——
    /// 若哪天有人把 `dummy_password_hash()` 换成空串，这条会立刻变红。
    #[test]
    fn 假哈希路径与真哈希路径耗时同量级() {
        use std::time::Instant;

        let real = hash_password("hunter2hunter2").unwrap();
        let dummy = dummy_password_hash();
        // 预热：把 OnceLock 初始化与首次分配的开销排除在计时之外。
        assert!(!verify_password("x_wrong_password", dummy));

        let t0 = Instant::now();
        assert!(!verify_password("x_wrong_password", &real));
        let 真 = t0.elapsed();

        let t1 = Instant::now();
        assert!(!verify_password("x_wrong_password", dummy));
        let 假 = t1.elapsed();

        let 比值 = 真.as_secs_f64().max(1e-9) / 假.as_secs_f64().max(1e-9);
        assert!(
            (0.2..=5.0).contains(&比值),
            "两条路径耗时相差过大：真 {真:?} / 假 {假:?}（比值 {比值:.2}）"
        );
    }
}
