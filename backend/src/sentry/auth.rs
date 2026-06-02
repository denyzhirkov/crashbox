//! Extract `sentry_key` (= project public key) from an SDK request.
//!
//! Sentry SDKs send credentials in one of two ways:
//! - `X-Sentry-Auth: Sentry sentry_version=7, sentry_key=PUBLIC_KEY, sentry_client=...`
//! - Query string: `?sentry_key=PUBLIC_KEY&sentry_version=7`
//!
//! For MVP we only care about `sentry_key`; the version and client name are accepted but ignored.

pub fn extract_sentry_key(auth_header: Option<&str>, query: Option<&str>) -> Option<String> {
    if let Some(h) = auth_header {
        if let Some(k) = parse_sentry_auth(h) {
            return Some(k);
        }
    }
    if let Some(q) = query {
        return parse_query_key(q);
    }
    None
}

fn parse_sentry_auth(header: &str) -> Option<String> {
    // Header may start with "Sentry " (case-insensitive); SDKs sometimes omit it.
    let body = header
        .strip_prefix("Sentry ")
        .or_else(|| header.strip_prefix("sentry "))
        .unwrap_or(header);

    for raw in body.split(',') {
        let pair = raw.trim();
        if let Some(rest) = pair.strip_prefix("sentry_key=") {
            return Some(rest.trim_matches('"').to_string());
        }
    }
    None
}

fn parse_query_key(query: &str) -> Option<String> {
    for raw in query.split('&') {
        if let Some(rest) = raw.strip_prefix("sentry_key=") {
            return Some(rest.to_string());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_header() {
        let h = "Sentry sentry_version=7, sentry_key=abcdef, sentry_client=sentry.javascript.node/7.0.0";
        assert_eq!(extract_sentry_key(Some(h), None).as_deref(), Some("abcdef"));
    }

    #[test]
    fn parses_header_without_prefix() {
        let h = "sentry_key=xyz, sentry_version=7";
        assert_eq!(extract_sentry_key(Some(h), None).as_deref(), Some("xyz"));
    }

    #[test]
    fn falls_back_to_query() {
        let q = "sentry_version=7&sentry_key=fromq&sentry_client=foo";
        assert_eq!(extract_sentry_key(None, Some(q)).as_deref(), Some("fromq"));
    }

    #[test]
    fn returns_none_when_missing() {
        assert!(extract_sentry_key(None, None).is_none());
        assert!(extract_sentry_key(Some("Sentry sentry_version=7"), None).is_none());
    }
}
