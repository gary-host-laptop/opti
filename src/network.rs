use std::path::PathBuf;
use std::time::Duration;

/// Download the body of an http(s) URL into memory.
pub fn fetch_url(url: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    let resp = ureq::get(url)
        .timeout(Duration::from_secs(30))
        .call()
        .map_err(|e| format!("{url}: {e}"))?;
    use std::io::Read;
    resp.into_reader()
        .read_to_end(&mut body)
        .map_err(|e| format!("{url}: read error: {e}"))?;
    Ok(body)
}

use base64::Engine;

/// Decode a `data:` URI (e.g. `data:image/png;base64,AAAA...`) into raw bytes.
/// Returns `None` if the string is not a base64 data URI.
pub fn decode_data_uri(s: &str) -> Option<Vec<u8>> {
    let rest = s.strip_prefix("data:")?;
    let (_, b64) = rest.split_once(',')?;
    if !s.contains(";base64") {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .ok()
}

/// Encode bytes as a `data:` URI with the given MIME type.
pub fn encode_data_uri(mime: &str, bytes: &[u8]) -> String {
    let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
    format!("data:{mime};base64,{b64}")
}

/// Write raw bytes to a temp file (so GStreamer can seek remote blobs).
pub fn write_temp_file(bytes: &[u8], suffix: &str) -> Result<PathBuf, String> {
    let dir = std::env::temp_dir();
    let name = format!("opti_{}_{}.{}", std::process::id(), unique_token(), suffix);
    let path = dir.join(name);
    std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

fn unique_token() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_uri_roundtrip() {
        let bytes = vec![1u8, 2, 3, 4, 255, 128, 0];
        let uri = encode_data_uri("image/png", &bytes);
        assert!(uri.starts_with("data:image/png;base64,"));
        assert_eq!(decode_data_uri(&uri).unwrap(), bytes);
        assert_eq!(decode_data_uri("data:image/png;base64,!!!not-valid"), None);
        assert_eq!(decode_data_uri("https://example.com/x.png"), None);
    }

    #[test]
    fn temp_file_roundtrip() {
        let path = write_temp_file(b"hello", "bin").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    #[ignore = "requires network"]
    fn fetch_live_url() {
        let bytes = fetch_url("https://example.com/").unwrap();
        assert!(bytes.len() > 100);
        let _ = decode_data_uri("");
    }
}