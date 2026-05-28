//! Issue fingerprinting (§10 of master prompt).
//!
//! Order of preference:
//! 1. `event.fingerprint` — explicit, wins outright.
//! 2. Exception: `platform | type | normalize(value) | top_frame_signature`.
//! 3. Message: `platform | normalize(message)`.
//! 4. Fallback: `platform | transaction|logger|event_id`.
//!
//! We do not try to bit-match Sentry's grouping algorithm — see `docs/protocol.md`.

use sha1::{Digest, Sha1};

use crate::sentry::normalize::{normalize_message, NormalizedEvent};

pub fn fingerprint(ev: &NormalizedEvent) -> String {
    let platform = ev.platform.as_deref().unwrap_or("unknown");

    if let Some(custom) = &ev.custom_fingerprint {
        return sha1_hex(&format!("custom|{platform}|{}", custom.join("|")));
    }

    if let Some(exc) = &ev.exception {
        if exc.ty.is_some() || exc.value.is_some() || exc.top_frame.is_some() {
            let key = format!(
                "exception|{platform}|{ty}|{val}|{frame}",
                ty = exc.ty.as_deref().unwrap_or(""),
                val = normalize_message(exc.value.as_deref().unwrap_or("")),
                frame = exc.top_frame.as_deref().unwrap_or(""),
            );
            return sha1_hex(&key);
        }
    }

    if let Some(msg) = &ev.message {
        let key = format!("message|{platform}|{}", normalize_message(msg));
        return sha1_hex(&key);
    }

    let last_resort = ev
        .transaction_name
        .as_deref()
        .or(ev.logger.as_deref())
        .or(ev.event_id.as_deref())
        .unwrap_or("");
    sha1_hex(&format!("fallback|{platform}|{last_resort}"))
}

fn sha1_hex(input: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sentry::normalize::{ExceptionInfo, NormalizedEvent};

    fn ev_exc(platform: &str, ty: &str, value: &str, frame: &str) -> NormalizedEvent {
        NormalizedEvent {
            platform: Some(platform.to_string()),
            exception: Some(ExceptionInfo {
                ty: Some(ty.to_string()),
                value: Some(value.to_string()),
                top_frame: Some(frame.to_string()),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn same_exception_groups_together() {
        let a = ev_exc("node", "TypeError", "x is undefined", "f@:a.js:1");
        let b = ev_exc("node", "TypeError", "x is undefined", "f@:a.js:1");
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn different_exception_type_groups_separately() {
        let a = ev_exc("node", "TypeError", "x", "f@:a.js:1");
        let b = ev_exc("node", "RangeError", "x", "f@:a.js:1");
        assert_ne!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn exception_value_with_variable_id_groups_together() {
        // The numeric id should be normalized away, so these two group together.
        let a = ev_exc(
            "node",
            "DbError",
            "row 12345678 not found",
            "f@:repo.js:1",
        );
        let b = ev_exc(
            "node",
            "DbError",
            "row 98765432 not found",
            "f@:repo.js:1",
        );
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn custom_fingerprint_wins() {
        let mut a = ev_exc("node", "Foo", "v1", "frame1");
        let mut b = ev_exc("node", "Bar", "v2", "frame2");
        a.custom_fingerprint = Some(vec!["bucket-A".into()]);
        b.custom_fingerprint = Some(vec!["bucket-A".into()]);
        assert_eq!(fingerprint(&a), fingerprint(&b));
    }

    #[test]
    fn message_only_grouping() {
        let mut a = NormalizedEvent::default();
        a.platform = Some("python".into());
        a.message = Some("connection timed out after 5000ms".into());
        let mut b = a.clone();
        b.message = Some("connection timed out after 5000ms".into());
        let mut c = a.clone();
        c.message = Some("totally different error".into());

        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }

    #[test]
    fn fallback_when_nothing() {
        let a = NormalizedEvent {
            platform: Some("ruby".into()),
            transaction_name: Some("GET /widgets".into()),
            ..Default::default()
        };
        let b = a.clone();
        let mut c = a.clone();
        c.transaction_name = Some("POST /widgets".into());
        assert_eq!(fingerprint(&a), fingerprint(&b));
        assert_ne!(fingerprint(&a), fingerprint(&c));
    }
}
