//! `Content-Encoding` decoding for ingest bodies (envelope now, legacy `/store/` later).
//!
//! Decompression is bounded: output is capped at the same limit the raw body obeys, so a
//! small compressed body cannot balloon past `CRASHBOX_MAX_ENVELOPE_BYTES` (zip-bomb
//! protection). Callers decode only after DSN auth and rate limiting, so unauthenticated
//! traffic never reaches the decompressor.

use std::io::Read;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum DecodeError {
    #[error("unsupported Content-Encoding: {0}")]
    Unsupported(String),
    #[error("decompressed body exceeds the size limit")]
    TooLarge,
    #[error("malformed compressed body: {0}")]
    Corrupt(#[from] std::io::Error),
}

/// Decode `body` according to a `Content-Encoding` header value.
///
/// Returns `None` for `identity`/empty (the body is usable as-is), `Some(decoded)` for a
/// supported compression scheme. Multi-encoding chains (`gzip, zstd`) are not supported and
/// map to [`DecodeError::Unsupported`].
pub fn decode(encoding: &str, body: &[u8], limit: usize) -> Result<Option<Vec<u8>>, DecodeError> {
    match encoding.trim().to_ascii_lowercase().as_str() {
        "" | "identity" => Ok(None),
        "gzip" | "x-gzip" => read_capped(flate2::read::GzDecoder::new(body), limit).map(Some),
        // Sentry SDKs send zlib-wrapped deflate (RFC 1950), not a raw deflate stream.
        "deflate" => read_capped(flate2::read::ZlibDecoder::new(body), limit).map(Some),
        "zstd" => read_capped(zstd::stream::read::Decoder::new(body)?, limit).map(Some),
        other => Err(DecodeError::Unsupported(other.to_string())),
    }
}

fn read_capped<R: Read>(reader: R, limit: usize) -> Result<Vec<u8>, DecodeError> {
    let mut out = Vec::new();
    reader.take(limit as u64 + 1).read_to_end(&mut out)?;
    if out.len() > limit {
        return Err(DecodeError::TooLarge);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).unwrap();
        enc.finish().unwrap()
    }

    #[test]
    fn identity_and_empty_pass_through() {
        assert!(decode("", b"raw", 100).unwrap().is_none());
        assert!(decode("identity", b"raw", 100).unwrap().is_none());
        assert!(decode(" Identity ", b"raw", 100).unwrap().is_none());
    }

    #[test]
    fn gzip_round_trip() {
        let out = decode("gzip", &gzip(b"hello envelope"), 100).unwrap();
        assert_eq!(out.as_deref(), Some(&b"hello envelope"[..]));
        let out = decode("x-gzip", &gzip(b"hello"), 100).unwrap();
        assert_eq!(out.as_deref(), Some(&b"hello"[..]));
    }

    #[test]
    fn deflate_round_trip() {
        let out = decode("deflate", &zlib(b"zlib body"), 100).unwrap();
        assert_eq!(out.as_deref(), Some(&b"zlib body"[..]));
    }

    #[test]
    fn zstd_round_trip() {
        let compressed = zstd::encode_all(&b"zstd body"[..], 0).unwrap();
        let out = decode("zstd", &compressed, 100).unwrap();
        assert_eq!(out.as_deref(), Some(&b"zstd body"[..]));
    }

    #[test]
    fn encoding_value_is_case_insensitive_and_trimmed() {
        let out = decode(" GZIP ", &gzip(b"x"), 100).unwrap();
        assert_eq!(out.as_deref(), Some(&b"x"[..]));
    }

    #[test]
    fn bomb_is_capped_at_limit() {
        let big = vec![0_u8; 1_000_000];
        let compressed = gzip(&big); // ~1 KB compressed
        assert!(compressed.len() < 10_000);
        assert!(matches!(
            decode("gzip", &compressed, 4096),
            Err(DecodeError::TooLarge)
        ));
    }

    #[test]
    fn exactly_at_limit_is_accepted() {
        let data = vec![7_u8; 4096];
        let out = decode("gzip", &gzip(&data), 4096).unwrap();
        assert_eq!(out.unwrap().len(), 4096);
    }

    #[test]
    fn unsupported_encoding_is_rejected() {
        assert!(matches!(
            decode("br", b"whatever", 100),
            Err(DecodeError::Unsupported(_))
        ));
        assert!(matches!(
            decode("gzip, zstd", b"chained", 100),
            Err(DecodeError::Unsupported(_))
        ));
    }

    #[test]
    fn garbage_stream_is_corrupt_not_panic() {
        assert!(matches!(
            decode("gzip", b"definitely not gzip", 100),
            Err(DecodeError::Corrupt(_))
        ));
        assert!(matches!(
            decode("deflate", b"nope", 100),
            Err(DecodeError::Corrupt(_))
        ));
        // zstd validates magic bytes at decoder construction.
        assert!(decode("zstd", b"nope", 100).is_err());
    }
}
