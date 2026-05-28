//! Sentry envelope parser.
//!
//! Envelope format (line-oriented):
//! ```text
//! {envelope_header_json}\n
//! {item_header_json}\n
//! {item_payload}\n
//! {item_header_json}\n
//! {item_payload}\n
//! ...
//! ```
//!
//! Each item header is JSON with at least `type`. If `length` is present, the payload is exactly
//! that many bytes; otherwise the payload runs until the next `\n` (or end of buffer).
//!
//! Robustness rules (MVP):
//! - Empty envelope → `EnvelopeError::Empty`.
//! - Invalid envelope/item header JSON → `EnvelopeError::BadHeader`.
//! - Declared `length` past end of buffer → `EnvelopeError::TruncatedPayload`.
//! - Otherwise parse what we can; unknown item `type` values are kept as-is for the caller.

use serde::Deserialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EnvelopeError {
    #[error("empty envelope")]
    Empty,
    #[error("invalid envelope header json: {0}")]
    BadHeader(serde_json::Error),
    #[error("invalid item header json at byte {at}: {source}")]
    BadItemHeader {
        at: usize,
        #[source]
        source: serde_json::Error,
    },
    #[error("item payload truncated at byte {at} (declared length {length})")]
    TruncatedPayload { at: usize, length: usize },
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct EnvelopeHeader {
    #[serde(default)]
    pub event_id: Option<String>,
    #[serde(default)]
    pub dsn: Option<String>,
    #[serde(default)]
    pub sent_at: Option<String>,
    // Keep the rest available for diagnostics.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Deserialize, Clone, Default)]
pub struct ItemHeader {
    #[serde(default, rename = "type")]
    pub ty: Option<String>,
    #[serde(default)]
    pub length: Option<usize>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug)]
pub struct Item {
    pub header: ItemHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug)]
pub struct Envelope {
    pub header: EnvelopeHeader,
    pub items: Vec<Item>,
}

pub fn parse(bytes: &[u8]) -> Result<Envelope, EnvelopeError> {
    if bytes.is_empty() {
        return Err(EnvelopeError::Empty);
    }

    let (header_line, mut rest_off) = read_line(bytes, 0);
    if header_line.is_empty() {
        return Err(EnvelopeError::Empty);
    }
    let header: EnvelopeHeader =
        serde_json::from_slice(header_line).map_err(EnvelopeError::BadHeader)?;

    let mut items = Vec::new();
    while rest_off < bytes.len() {
        // Skip any blank separator lines.
        while rest_off < bytes.len() && bytes[rest_off] == b'\n' {
            rest_off += 1;
        }
        if rest_off >= bytes.len() {
            break;
        }

        let item_start = rest_off;
        let (item_header_line, after_header) = read_line(bytes, rest_off);
        if item_header_line.is_empty() {
            // Trailing whitespace / EOF.
            break;
        }
        let item_header: ItemHeader =
            serde_json::from_slice(item_header_line).map_err(|e| EnvelopeError::BadItemHeader {
                at: item_start,
                source: e,
            })?;
        rest_off = after_header;

        let payload = if let Some(len) = item_header.length {
            if rest_off + len > bytes.len() {
                return Err(EnvelopeError::TruncatedPayload {
                    at: rest_off,
                    length: len,
                });
            }
            let p = bytes[rest_off..rest_off + len].to_vec();
            rest_off += len;
            // Optional trailing newline after a sized payload.
            if rest_off < bytes.len() && bytes[rest_off] == b'\n' {
                rest_off += 1;
            }
            p
        } else {
            let (line, next) = read_line(bytes, rest_off);
            rest_off = next;
            line.to_vec()
        };

        items.push(Item {
            header: item_header,
            payload,
        });
    }

    Ok(Envelope { header, items })
}

/// Read a line (up to next `\n`), return (slice without trailing \n, offset after the \n).
fn read_line(buf: &[u8], start: usize) -> (&[u8], usize) {
    if start >= buf.len() {
        return (&[], start);
    }
    if let Some(rel) = buf[start..].iter().position(|&b| b == b'\n') {
        (&buf[start..start + rel], start + rel + 1)
    } else {
        (&buf[start..], buf.len())
    }
}

impl Item {
    pub fn is_event(&self) -> bool {
        matches!(self.header.ty.as_deref(), Some("event"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(s: &str) -> Vec<u8> {
        s.as_bytes().to_vec()
    }

    #[test]
    fn rejects_empty() {
        assert!(matches!(parse(b""), Err(EnvelopeError::Empty)));
    }

    #[test]
    fn parses_single_event_with_length() {
        let payload = r#"{"message":"hello"}"#;
        let raw = format!(
            "{{\"event_id\":\"abc\"}}\n{{\"type\":\"event\",\"length\":{}}}\n{}\n",
            payload.len(),
            payload
        );
        let env = parse(&body(&raw)).expect("parse");
        assert_eq!(env.header.event_id.as_deref(), Some("abc"));
        assert_eq!(env.items.len(), 1);
        assert!(env.items[0].is_event());
        assert_eq!(env.items[0].payload, payload.as_bytes());
    }

    #[test]
    fn parses_event_without_length() {
        let raw = "{}\n{\"type\":\"event\"}\n{\"message\":\"x\"}\n";
        let env = parse(&body(raw)).expect("parse");
        assert_eq!(env.items.len(), 1);
        assert_eq!(env.items[0].payload, br#"{"message":"x"}"#);
    }

    #[test]
    fn parses_multiple_items_mixed() {
        let raw = "{}\n\
                   {\"type\":\"event\",\"length\":2}\n\
                   {}\n\
                   {\"type\":\"attachment\"}\n\
                   binary-or-text\n";
        let env = parse(&body(raw)).expect("parse");
        assert_eq!(env.items.len(), 2);
        assert_eq!(env.items[0].header.ty.as_deref(), Some("event"));
        assert_eq!(env.items[1].header.ty.as_deref(), Some("attachment"));
        assert_eq!(env.items[1].payload, b"binary-or-text");
    }

    #[test]
    fn bad_envelope_header() {
        let raw = "not-json\n{\"type\":\"event\"}\n{}\n";
        assert!(matches!(parse(&body(raw)), Err(EnvelopeError::BadHeader(_))));
    }

    #[test]
    fn bad_item_header() {
        let raw = "{}\nnot-json\npayload\n";
        assert!(matches!(
            parse(&body(raw)),
            Err(EnvelopeError::BadItemHeader { .. })
        ));
    }

    #[test]
    fn truncated_payload_when_length_lies() {
        let raw = "{}\n{\"type\":\"event\",\"length\":999}\nshort\n";
        assert!(matches!(
            parse(&body(raw)),
            Err(EnvelopeError::TruncatedPayload { .. })
        ));
    }

    #[test]
    fn unknown_item_type_kept() {
        let raw = "{}\n{\"type\":\"weird-thing\"}\n{}\n";
        let env = parse(&body(raw)).expect("parse");
        assert_eq!(env.items[0].header.ty.as_deref(), Some("weird-thing"));
        assert!(!env.items[0].is_event());
    }
}
