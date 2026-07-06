//! Personal API tokens — generation and hashing.
//!
//! A token is `cbx_` + 32 hex chars (128 random bits from the OS CSPRNG). Only its SHA-256
//! is stored; the plaintext is returned exactly once at creation. SHA-256 (not argon2) is
//! correct here: argon2's cost exists to protect low-entropy passwords, while a 128-bit
//! random token can't be brute-forced anyway — and a deterministic hash gives an O(1)
//! indexed lookup on every authenticated request.

use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

pub const TOKEN_PREFIX: &str = "cbx_";
/// Chars of the plaintext kept for identification in list views ("cbx_a1b2c3").
const DISPLAY_PREFIX_LEN: usize = 10;

pub fn generate() -> String {
    let mut bytes = [0u8; 16];
    OsRng.fill_bytes(&mut bytes);
    format!("{TOKEN_PREFIX}{}", hex::encode(bytes))
}

pub fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub fn display_prefix(token: &str) -> String {
    token.chars().take(DISPLAY_PREFIX_LEN).collect()
}

/// Extract the token from an `Authorization` header value, tolerating case in the scheme.
/// Returns `None` for anything that isn't `Bearer cbx_…` — callers answer uniform 401.
pub fn from_authorization_header(value: &str) -> Option<&str> {
    let rest = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?;
    let token = rest.trim();
    token.starts_with(TOKEN_PREFIX).then_some(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_unique_and_well_formed() {
        let a = generate();
        let b = generate();
        assert_ne!(a, b);
        assert!(a.starts_with(TOKEN_PREFIX));
        assert_eq!(a.len(), TOKEN_PREFIX.len() + 32);
    }

    #[test]
    fn hash_is_stable_and_not_the_plaintext() {
        let t = generate();
        assert_eq!(hash(&t), hash(&t));
        assert_ne!(hash(&t), t);
        assert_eq!(hash(&t).len(), 64);
    }

    #[test]
    fn header_parsing() {
        assert_eq!(
            from_authorization_header("Bearer cbx_abc123"),
            Some("cbx_abc123")
        );
        assert_eq!(
            from_authorization_header("bearer cbx_abc123"),
            Some("cbx_abc123")
        );
        assert_eq!(from_authorization_header("Bearer sk-something"), None);
        assert_eq!(from_authorization_header("Basic dXNlcjpwdw=="), None);
        assert_eq!(from_authorization_header("cbx_abc123"), None);
    }
}
