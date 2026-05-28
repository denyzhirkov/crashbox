use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum PasswordError {
    #[error("password hashing failed: {0}")]
    Hash(String),
    #[error("password verification failed")]
    Verify,
}

pub fn hash_password(plain: &str) -> Result<String, PasswordError> {
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map_err(|e| PasswordError::Hash(e.to_string()))?
        .to_string();
    Ok(hash)
}

pub fn verify_password(plain: &str, hash: &str) -> Result<bool, PasswordError> {
    let parsed = PasswordHash::new(hash).map_err(|e| PasswordError::Hash(e.to_string()))?;
    Ok(Argon2::default()
        .verify_password(plain.as_bytes(), &parsed)
        .is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_ok() {
        let h = hash_password("hunter2").expect("hash");
        assert!(verify_password("hunter2", &h).expect("verify"));
        assert!(!verify_password("wrong", &h).expect("verify"));
    }

    #[test]
    fn rejects_garbage_hash() {
        let err = verify_password("x", "not-a-hash").expect_err("must fail to parse");
        assert!(matches!(err, PasswordError::Hash(_)));
    }
}
