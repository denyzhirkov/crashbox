//! Live Logs domain: an ephemeral, RAM-only log record and the parsing that normalizes the two
//! accepted wire formats into it. Nothing here touches the database — logs are never persisted.
//!
//! Two ingest formats are normalized into [`LogRecord`]:
//! - **Loose** (our own `/logs` endpoint): a flat JSON object per record, e.g.
//!   `{"level":"info","message":"...","logger":"auth","ts":"2026-..","attrs":{...}}`.
//! - **Sentry `log` envelope item**: an OTel-style batch
//!   `{"items":[{"timestamp":1.7e9,"level":"info","body":"...","attributes":{"k":{"value":...}}}]}`.

pub mod hub;

pub use hub::{LiveLogHub, SubscribeError, Subscription};

use chrono::{TimeZone, Utc};
use serde::Serialize;
use serde_json::{Map, Value};

/// Severity of a live log line. Distinct from event levels — logs are informational by nature and
/// default to `Info` when the producer omits or sends an unknown level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl LogLevel {
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "trace" => Self::Trace,
            "debug" => Self::Debug,
            "warn" | "warning" => Self::Warn,
            "error" | "err" => Self::Error,
            "fatal" | "critical" => Self::Fatal,
            _ => Self::Info,
        }
    }

    /// Severity rank, ascending. Used by the stream's `level` floor filter.
    pub fn rank(self) -> u8 {
        match self {
            Self::Trace => 0,
            Self::Debug => 1,
            Self::Info => 2,
            Self::Warn => 3,
            Self::Error => 4,
            Self::Fatal => 5,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LogRecord {
    pub ts: String,
    pub level: LogLevel,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(skip_serializing_if = "Map::is_empty")]
    pub attrs: Map<String, Value>,
}

/// Keys lifted into typed fields — everything else on a loose record falls through to `attrs`.
const LOOSE_RESERVED: &[&str] = &[
    "level",
    "message",
    "msg",
    "body",
    "logger",
    "source",
    "ts",
    "timestamp",
    "trace_id",
];

impl LogRecord {
    /// Parse one record from the loose `/logs` format. Returns `None` only when the value is not a
    /// JSON object — a missing message or level is tolerated, not rejected.
    pub fn from_loose(value: &Value, message_max_bytes: usize) -> Option<Self> {
        let obj = value.as_object()?;
        let message = obj
            .get("message")
            .or_else(|| obj.get("msg"))
            .or_else(|| obj.get("body"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let level = obj
            .get("level")
            .and_then(Value::as_str)
            .map_or(LogLevel::Info, LogLevel::parse);
        let logger = obj
            .get("logger")
            .or_else(|| obj.get("source"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let ts = parse_ts(obj.get("ts").or_else(|| obj.get("timestamp")))
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let trace_id = obj
            .get("trace_id")
            .and_then(Value::as_str)
            .map(str::to_string);

        let mut attrs = Map::new();
        for (k, v) in obj {
            if !LOOSE_RESERVED.contains(&k.as_str()) {
                attrs.insert(k.clone(), v.clone());
            }
        }

        Some(Self {
            ts,
            level,
            message: truncate(message, message_max_bytes),
            logger,
            trace_id,
            attrs,
        })
    }

    /// Parse a Sentry `log` envelope item payload into zero or more records. Resilient: a malformed
    /// batch yields an empty vec, a malformed entry is skipped.
    pub fn from_sentry_batch(payload: &Value, message_max_bytes: usize) -> Vec<Self> {
        let Some(items) = payload.get("items").and_then(Value::as_array) else {
            return Vec::new();
        };
        items
            .iter()
            .filter_map(|item| Self::from_sentry_entry(item, message_max_bytes))
            .collect()
    }

    fn from_sentry_entry(value: &Value, message_max_bytes: usize) -> Option<Self> {
        let obj = value.as_object()?;
        let message = obj
            .get("body")
            .or_else(|| obj.get("message"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let level = obj
            .get("level")
            .and_then(Value::as_str)
            .map_or(LogLevel::Info, LogLevel::parse);
        let ts = parse_ts(obj.get("timestamp").or_else(|| obj.get("ts")))
            .unwrap_or_else(|| Utc::now().to_rfc3339());
        let trace_id = obj
            .get("trace_id")
            .and_then(Value::as_str)
            .map(str::to_string);

        // Sentry attributes are typed wrappers: `{"value": x, "type": "string"}`. Unwrap to the
        // bare value; fall back to the whole node if it doesn't follow the shape.
        let mut attrs = Map::new();
        if let Some(map) = obj.get("attributes").and_then(Value::as_object) {
            for (k, v) in map {
                let unwrapped = v.get("value").cloned().unwrap_or_else(|| v.clone());
                attrs.insert(k.clone(), unwrapped);
            }
        }

        Some(Self {
            ts,
            level,
            message: truncate(message, message_max_bytes),
            logger: None,
            trace_id,
            attrs,
        })
    }
}

/// Accept either an ISO-8601 string or an epoch-seconds number (Sentry sends floats).
fn parse_ts(value: Option<&Value>) -> Option<String> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => {
            let secs = n.as_f64()?;
            let nanos = (secs.fract().abs() * 1e9).round() as u32;
            Utc.timestamp_opt(secs.trunc() as i64, nanos)
                .single()
                .map(|dt| dt.to_rfc3339())
        }
        _ => None,
    }
}

/// Truncate to at most `max` bytes on a UTF-8 char boundary, appending an ellipsis when cut.
fn truncate(mut s: String, max: usize) -> String {
    if s.len() <= max {
        return s;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push('…');
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loose_parses_typed_fields_and_falls_through_to_attrs() {
        let v = json!({
            "level": "WARN",
            "message": "disk almost full",
            "logger": "storage",
            "ts": "2026-06-02T10:00:00Z",
            "trace_id": "abc",
            "free_mb": 12
        });
        let rec = LogRecord::from_loose(&v, 1024).expect("object parses");
        assert_eq!(rec.level, LogLevel::Warn);
        assert_eq!(rec.message, "disk almost full");
        assert_eq!(rec.logger.as_deref(), Some("storage"));
        assert_eq!(rec.trace_id.as_deref(), Some("abc"));
        assert_eq!(rec.attrs.get("free_mb"), Some(&json!(12)));
        assert!(!rec.attrs.contains_key("level"));
    }

    #[test]
    fn loose_tolerates_missing_message_and_level() {
        let v = json!({ "foo": "bar" });
        let rec = LogRecord::from_loose(&v, 1024).expect("object parses");
        assert_eq!(rec.level, LogLevel::Info);
        assert_eq!(rec.message, "");
        assert_eq!(rec.attrs.get("foo"), Some(&json!("bar")));
    }

    #[test]
    fn loose_rejects_non_object() {
        assert!(LogRecord::from_loose(&json!("nope"), 1024).is_none());
        assert!(LogRecord::from_loose(&json!([1, 2, 3]), 1024).is_none());
    }

    #[test]
    fn sentry_batch_unwraps_typed_attributes_and_epoch_ts() {
        let payload = json!({
            "items": [
                {
                    "timestamp": 1_700_000_000.5,
                    "level": "error",
                    "body": "boom",
                    "trace_id": "t1",
                    "attributes": { "user_id": { "value": 42, "type": "integer" } }
                },
                { "not": "valid-but-still-an-object" }
            ]
        });
        let recs = LogRecord::from_sentry_batch(&payload, 1024);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0].level, LogLevel::Error);
        assert_eq!(recs[0].message, "boom");
        assert_eq!(recs[0].attrs.get("user_id"), Some(&json!(42)));
        assert!(recs[0].ts.starts_with("2023-11-14"));
    }

    #[test]
    fn sentry_batch_without_items_is_empty() {
        assert!(LogRecord::from_sentry_batch(&json!({}), 1024).is_empty());
    }

    #[test]
    fn truncate_respects_char_boundaries() {
        let s = "héllo wörld".to_string();
        let out = truncate(s, 3);
        assert!(out.ends_with('…'));
        assert!(out.len() <= 3 + '…'.len_utf8() + 1);
    }
}
