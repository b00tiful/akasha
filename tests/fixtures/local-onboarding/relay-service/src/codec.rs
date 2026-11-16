use crate::config::MAX_PAYLOAD_BYTES;
/// Codec rejects empty payloads and payloads above the byte limit; valid text is preserved exactly.
pub fn encode(payload: &str) -> Option<Vec<u8>> {
    if payload.is_empty() || payload.len() > MAX_PAYLOAD_BYTES {
        None
    } else {
        Some(payload.as_bytes().to_vec())
    }
}
