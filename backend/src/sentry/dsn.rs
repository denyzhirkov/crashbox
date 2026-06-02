use thiserror::Error;

#[derive(Debug, Error)]
pub enum DsnError {
    #[error("dsn must start with http:// or https://")]
    Scheme,
    #[error("dsn missing public key")]
    MissingPublicKey,
    #[error("dsn missing host")]
    MissingHost,
    #[error("dsn missing project id")]
    MissingProjectId,
    #[error("invalid project id: {0}")]
    InvalidProjectId(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dsn {
    pub scheme: String,
    pub public_key: String,
    pub host: String,
    pub project_id: i64,
}

impl Dsn {
    /// Build a DSN string from a configured public_url and project credentials.
    /// public_url is expected as `http(s)://host[:port]`.
    pub fn build(public_url: &str, public_key: &str, project_id: i64) -> String {
        let trimmed = public_url.trim_end_matches('/');
        if let Some(rest) = trimmed.strip_prefix("https://") {
            format!("https://{public_key}@{rest}/{project_id}")
        } else if let Some(rest) = trimmed.strip_prefix("http://") {
            format!("http://{public_key}@{rest}/{project_id}")
        } else {
            format!("http://{public_key}@{trimmed}/{project_id}")
        }
    }

    /// Parse `http(s)://public_key@host[:port]/project_id`.
    pub fn parse(raw: &str) -> Result<Self, DsnError> {
        let (scheme, rest) = if let Some(r) = raw.strip_prefix("https://") {
            ("https", r)
        } else if let Some(r) = raw.strip_prefix("http://") {
            ("http", r)
        } else {
            return Err(DsnError::Scheme);
        };

        let (public_key, after_at) = rest.split_once('@').ok_or(DsnError::MissingPublicKey)?;
        if public_key.is_empty() {
            return Err(DsnError::MissingPublicKey);
        }

        let (host, project) = after_at.split_once('/').ok_or(DsnError::MissingProjectId)?;
        if host.is_empty() {
            return Err(DsnError::MissingHost);
        }
        // Project segment may have trailing slash or query.
        let project_clean = project
            .trim_end_matches('/')
            .split('?')
            .next()
            .unwrap_or("");
        if project_clean.is_empty() {
            return Err(DsnError::MissingProjectId);
        }
        let project_id: i64 = project_clean
            .parse()
            .map_err(|_| DsnError::InvalidProjectId(project_clean.to_string()))?;

        Ok(Self {
            scheme: scheme.to_string(),
            public_key: public_key.to_string(),
            host: host.to_string(),
            project_id,
        })
    }
}

/// Mask the public key for non-bootstrap logging: keep first 4 + last 2 chars.
pub fn mask_public_key(key: &str) -> String {
    let len = key.chars().count();
    if len <= 8 {
        return "*".repeat(len);
    }
    let head: String = key.chars().take(4).collect();
    let tail: String = key.chars().skip(len - 2).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_basic() {
        let s = Dsn::build("http://localhost:8080", "abcdef", 1);
        assert_eq!(s, "http://abcdef@localhost:8080/1");
    }

    #[test]
    fn build_https_strips_trailing_slash() {
        let s = Dsn::build("https://crash.example.com/", "k", 42);
        assert_eq!(s, "https://k@crash.example.com/42");
    }

    #[test]
    fn parse_roundtrip() {
        let dsn = Dsn::parse("http://abcdef@localhost:8080/1").expect("parse");
        assert_eq!(dsn.scheme, "http");
        assert_eq!(dsn.public_key, "abcdef");
        assert_eq!(dsn.host, "localhost:8080");
        assert_eq!(dsn.project_id, 1);
    }

    #[test]
    fn parse_rejects_bad_scheme() {
        assert!(matches!(
            Dsn::parse("ftp://k@h/1").unwrap_err(),
            DsnError::Scheme
        ));
    }

    #[test]
    fn parse_rejects_missing_project() {
        assert!(matches!(
            Dsn::parse("http://k@h/").unwrap_err(),
            DsnError::MissingProjectId
        ));
    }

    #[test]
    fn mask_short_key() {
        assert_eq!(mask_public_key("abc"), "***");
    }

    #[test]
    fn mask_long_key() {
        assert_eq!(mask_public_key("abcdef1234"), "abcd…34");
    }
}
