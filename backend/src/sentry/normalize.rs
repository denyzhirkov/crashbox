//! Normalize raw Sentry event JSON into a typed `NormalizedEvent`.
//!
//! Only the fields needed for storage + grouping are extracted; the full original payload is kept
//! separately as `raw_json`. Missing/malformed fields default to `None`; this is a robustness
//! contract — ingestion must not fail because of a field shape difference between SDK versions.

use serde_json::Value;

#[derive(Debug, Default, Clone)]
pub struct NormalizedEvent {
    pub event_id: Option<String>,
    pub timestamp: Option<String>,
    pub platform: Option<String>,
    pub level: Option<String>,
    pub logger: Option<String>,
    pub transaction_name: Option<String>,
    pub environment: Option<String>,
    pub release: Option<String>,
    pub server_name: Option<String>,

    pub message: Option<String>,
    pub culprit: Option<String>,

    pub exception: Option<ExceptionInfo>,

    pub request_url: Option<String>,
    pub user_id: Option<String>,
    pub user_email: Option<String>,

    pub custom_fingerprint: Option<Vec<String>>,

    pub tags: Vec<(String, String)>,
    pub breadcrumbs: Vec<Breadcrumb>,
}

#[derive(Debug, Default, Clone)]
pub struct ExceptionInfo {
    pub ty: Option<String>,
    pub value: Option<String>,
    /// Best stack frame signature: `function@module:filename:lineno` of the topmost in-app frame
    /// (or the topmost frame, if none are marked in-app).
    pub top_frame: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Breadcrumb {
    pub timestamp: Option<String>,
    pub category: Option<String>,
    pub level: Option<String>,
    pub message: Option<String>,
    pub data_json: Option<String>,
}

/// Normalize once from a parsed JSON value. Never panics, never returns Err — fields just stay
/// None when the payload is unexpected.
pub fn from_value(v: &Value) -> NormalizedEvent {
    let mut ev = NormalizedEvent {
        event_id: str_field(v, "event_id"),
        timestamp: str_field(v, "timestamp"),
        platform: str_field(v, "platform"),
        level: str_field(v, "level"),
        logger: str_field(v, "logger"),
        transaction_name: str_field(v, "transaction"),
        environment: str_field(v, "environment"),
        release: str_field(v, "release"),
        server_name: str_field(v, "server_name"),
        message: extract_message(v),
        culprit: str_field(v, "culprit"),
        exception: extract_exception(v),
        request_url: v
            .get("request")
            .and_then(|r| r.get("url"))
            .and_then(Value::as_str)
            .map(str::to_string),
        user_id: v
            .get("user")
            .and_then(|u| u.get("id"))
            .map(stringify_scalar),
        user_email: v
            .get("user")
            .and_then(|u| u.get("email"))
            .and_then(Value::as_str)
            .map(str::to_string),
        custom_fingerprint: extract_custom_fingerprint(v),
        tags: extract_tags(v),
        breadcrumbs: extract_breadcrumbs(v),
    };

    // Default level to "error" when an exception is present, "info" otherwise — matches Sentry SDK
    // behavior closely enough for grouping/UX, and only when SDK omits the field.
    if ev.level.is_none() {
        ev.level = Some(
            if ev.exception.is_some() {
                "error"
            } else {
                "info"
            }
            .to_string(),
        );
    }
    ev
}

/// Build a short, human-readable title for the issue row. Prefers exception type+value, then
/// message, then a generic fallback.
pub fn title_for(ev: &NormalizedEvent) -> String {
    if let Some(exc) = &ev.exception {
        match (exc.ty.as_deref(), exc.value.as_deref()) {
            (Some(t), Some(v)) if !v.is_empty() => return clamp(&format!("{t}: {v}"), 200),
            (Some(t), _) => return clamp(t, 200),
            (None, Some(v)) => return clamp(v, 200),
            _ => {}
        }
    }
    if let Some(m) = &ev.message {
        return clamp(m, 200);
    }
    if let Some(t) = &ev.transaction_name {
        return clamp(t, 200);
    }
    "<unknown event>".to_string()
}

fn str_field(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn stringify_scalar(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => v.to_string(),
    }
}

fn clamp(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max - 1).collect();
    format!("{truncated}…")
}

/// Sentry events carry either a structured `{message: {formatted, message}}` or a plain string
/// in `message`. Accept both shapes.
fn extract_message(v: &Value) -> Option<String> {
    match v.get("message") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Object(o)) => o
            .get("formatted")
            .and_then(Value::as_str)
            .or_else(|| o.get("message").and_then(Value::as_str))
            .map(str::to_string),
        _ => None,
    }
}

fn extract_exception(v: &Value) -> Option<ExceptionInfo> {
    let exc = v.get("exception")?;
    let values = exc
        .get("values")
        .and_then(Value::as_array)
        .or_else(|| exc.as_array())?;
    let primary = values.last()?; // Sentry convention: the *last* exception in `values` is the
                                  // most recently raised (deepest cause first).

    let ty = primary
        .get("type")
        .and_then(Value::as_str)
        .map(str::to_string);
    let value = primary
        .get("value")
        .and_then(Value::as_str)
        .map(str::to_string);
    let top_frame = primary
        .get("stacktrace")
        .and_then(|s| s.get("frames"))
        .and_then(Value::as_array)
        .and_then(|frames| best_frame_signature(frames));

    if ty.is_none() && value.is_none() && top_frame.is_none() {
        return None;
    }
    Some(ExceptionInfo {
        ty,
        value,
        top_frame,
    })
}

/// Sentry orders frames bottom-up: last frame in the array is the topmost (most recent) call.
/// Prefer the topmost `in_app == true` frame; fall back to the topmost frame.
fn best_frame_signature(frames: &[Value]) -> Option<String> {
    let in_app = frames
        .iter()
        .rev()
        .find(|f| f.get("in_app").and_then(Value::as_bool).unwrap_or(false));
    let chosen = in_app.or_else(|| frames.last())?;
    Some(frame_signature(chosen))
}

fn frame_signature(frame: &Value) -> String {
    let func = frame
        .get("function")
        .and_then(Value::as_str)
        .unwrap_or("<anon>");
    let module = frame.get("module").and_then(Value::as_str).unwrap_or("");
    let filename = frame
        .get("filename")
        .or_else(|| frame.get("abs_path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let lineno = frame
        .get("lineno")
        .map(stringify_scalar)
        .unwrap_or_default();
    format!("{func}@{module}:{filename}:{lineno}")
}

fn extract_custom_fingerprint(v: &Value) -> Option<Vec<String>> {
    let arr = v.get("fingerprint").and_then(Value::as_array)?;
    let parts: Vec<String> = arr.iter().map(stringify_scalar).collect();
    // Sentry uses the literal "{{ default }}" placeholder to mean "use built-in grouping"; if the
    // array is *only* that placeholder, treat as no custom fingerprint.
    if parts.iter().all(|p| p == "{{ default }}") {
        return None;
    }
    Some(parts)
}

fn extract_tags(v: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let Some(tags) = v.get("tags") else {
        return out;
    };
    match tags {
        Value::Object(o) => {
            for (k, val) in o {
                if let Some(s) = val.as_str() {
                    out.push((k.clone(), s.to_string()));
                } else {
                    out.push((k.clone(), stringify_scalar(val)));
                }
            }
        }
        Value::Array(arr) => {
            // Some SDKs send tags as `[["key","value"], ...]`.
            for pair in arr {
                if let Some(pp) = pair.as_array() {
                    if pp.len() == 2 {
                        let k = pp[0].as_str().map(str::to_string);
                        let v_ = Some(stringify_scalar(&pp[1]));
                        if let (Some(k), Some(v_)) = (k, v_) {
                            out.push((k, v_));
                        }
                    }
                }
            }
        }
        _ => {}
    }
    out
}

fn extract_breadcrumbs(v: &Value) -> Vec<Breadcrumb> {
    // Breadcrumbs may be `{values: [...]}` or a bare array.
    let arr = v.get("breadcrumbs").and_then(|b| {
        b.get("values")
            .and_then(Value::as_array)
            .or_else(|| b.as_array())
    });
    let Some(arr) = arr else { return Vec::new() };

    arr.iter()
        .map(|b| Breadcrumb {
            timestamp: b.get("timestamp").map(stringify_scalar),
            category: b
                .get("category")
                .and_then(Value::as_str)
                .map(str::to_string),
            level: b.get("level").and_then(Value::as_str).map(str::to_string),
            message: b.get("message").and_then(Value::as_str).map(str::to_string),
            data_json: b.get("data").map(ToString::to_string),
        })
        .collect()
}

/// Normalize a free-form message for grouping. Replaces variable bits so the same logical error
/// with different IDs/hashes/etc. groups together.
pub fn normalize_message(input: &str) -> String {
    let trimmed = input.trim();

    let mut out = String::with_capacity(trimmed.len());
    let mut iter = trimmed.chars().peekable();
    let mut buf = String::new();

    while let Some(ch) = iter.next() {
        if ch.is_alphanumeric() || ch == '-' || ch == '_' {
            buf.push(ch);
            if iter.peek().is_some() {
                continue;
            }
        }
        // Flush the buffered "word" (or whatever) with substitutions, then handle the separator.
        if !buf.is_empty() {
            out.push_str(&substitute_token(&buf));
            buf.clear();
        }
        if !(ch.is_alphanumeric() || ch == '-' || ch == '_') {
            out.push(ch);
        }
    }

    // Collapse runs of whitespace.
    let collapsed = collapse_whitespace(&out);

    clamp(&collapsed, 500)
}

fn substitute_token(t: &str) -> String {
    if is_uuid_like(t) {
        return "<uuid>".to_string();
    }
    if is_long_hex(t) {
        return "<hex>".to_string();
    }
    if is_pure_long_number(t) {
        return "<num>".to_string();
    }
    t.to_string()
}

fn is_uuid_like(t: &str) -> bool {
    // Classic 8-4-4-4-12 hex layout.
    let parts: Vec<&str> = t.split('-').collect();
    if parts.len() != 5 {
        return false;
    }
    let expected = [8usize, 4, 4, 4, 12];
    if !parts.iter().zip(expected).all(|(p, n)| p.len() == n) {
        return false;
    }
    parts
        .iter()
        .all(|p| p.chars().all(|c| c.is_ascii_hexdigit()))
}

fn is_long_hex(t: &str) -> bool {
    t.len() >= 16 && t.chars().all(|c| c.is_ascii_hexdigit())
}

fn is_pure_long_number(t: &str) -> bool {
    t.len() >= 8 && t.chars().all(|c| c.is_ascii_digit())
}

fn collapse_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut prev_space = false;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            out.push(ch);
            prev_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_exception_basic() {
        let v = json!({
            "platform": "node",
            "exception": {
                "values": [
                    {
                        "type": "TypeError",
                        "value": "Cannot read property 'x' of undefined",
                        "stacktrace": {
                            "frames": [
                                {"function": "main", "filename": "/app/main.js", "lineno": 10, "in_app": false},
                                {"function": "handler", "filename": "/app/h.js", "lineno": 42, "in_app": true},
                            ]
                        }
                    }
                ]
            }
        });
        let ev = from_value(&v);
        assert_eq!(ev.platform.as_deref(), Some("node"));
        assert_eq!(ev.level.as_deref(), Some("error"));
        let exc = ev.exception.expect("exc");
        assert_eq!(exc.ty.as_deref(), Some("TypeError"));
        assert!(exc.value.as_deref().unwrap().contains("Cannot read"));
        let frame = exc.top_frame.expect("frame");
        assert!(frame.contains("handler"), "got: {frame}");
        assert!(frame.contains("h.js"));
    }

    #[test]
    fn message_string_or_object() {
        let plain = json!({"message": "boom"});
        assert_eq!(from_value(&plain).message.as_deref(), Some("boom"));

        let obj = json!({"message": {"formatted": "boom 42"}});
        assert_eq!(from_value(&obj).message.as_deref(), Some("boom 42"));
    }

    #[test]
    fn tags_object_and_array() {
        let o = json!({"tags": {"env": "prod", "rev": 123}});
        let t1 = from_value(&o).tags;
        assert!(t1.contains(&("env".to_string(), "prod".to_string())));
        assert!(t1.contains(&("rev".to_string(), "123".to_string())));

        let a = json!({"tags": [["a", "1"], ["b", 2]]});
        let t2 = from_value(&a).tags;
        assert!(t2.contains(&("a".to_string(), "1".to_string())));
        assert!(t2.contains(&("b".to_string(), "2".to_string())));
    }

    #[test]
    fn breadcrumbs_both_shapes() {
        let wrapped = json!({"breadcrumbs": {"values": [{"category": "ui", "message": "click"}]}});
        let bare = json!({"breadcrumbs": [{"category": "ui", "message": "click"}]});
        assert_eq!(from_value(&wrapped).breadcrumbs.len(), 1);
        assert_eq!(from_value(&bare).breadcrumbs.len(), 1);
    }

    #[test]
    fn custom_fingerprint_default_placeholder_ignored() {
        let v = json!({"fingerprint": ["{{ default }}"]});
        assert!(from_value(&v).custom_fingerprint.is_none());

        let v2 = json!({"fingerprint": ["my-bucket", "{{ default }}"]});
        assert_eq!(
            from_value(&v2).custom_fingerprint.as_deref(),
            Some(&["my-bucket".to_string(), "{{ default }}".to_string()][..])
        );
    }

    #[test]
    fn title_prefers_exception() {
        let v = json!({
            "message": "ignored",
            "exception": {"values": [{"type": "Foo", "value": "bar"}]},
        });
        assert_eq!(title_for(&from_value(&v)), "Foo: bar");
    }

    #[test]
    fn title_falls_back_to_message() {
        let v = json!({"message": "boom"});
        assert_eq!(title_for(&from_value(&v)), "boom");
    }

    #[test]
    fn normalize_message_replaces_variables() {
        let m = "user 550e8400-e29b-41d4-a716-446655440000 failed with id 1234567890";
        let n = normalize_message(m);
        assert_eq!(n, "user <uuid> failed with id <num>");
    }

    #[test]
    fn normalize_message_replaces_long_hex() {
        let n = normalize_message("hash abc123def4567890 mismatch");
        assert_eq!(n, "hash <hex> mismatch");
    }

    #[test]
    fn normalize_message_keeps_short_numbers() {
        let n = normalize_message("error 404 on path /a/b");
        assert!(n.contains("404"));
    }

    #[test]
    fn normalize_message_collapses_ws_and_trims() {
        let n = normalize_message("  boom   boom\n\tboom ");
        assert_eq!(n, "boom boom boom");
    }
}
