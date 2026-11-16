/// Relay accepts a maximum of 12 queued messages by default.
pub const QUEUE_CAPACITY: usize = 12;
/// Payloads larger than 256 bytes are rejected by the codec.
pub const MAX_PAYLOAD_BYTES: usize = 256;
