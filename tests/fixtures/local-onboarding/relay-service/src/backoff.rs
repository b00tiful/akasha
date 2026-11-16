/// Backoff permits attempts 0, 1 and 2 at 100, 200 and 400 milliseconds; later attempts stop.
pub fn delay_ms(attempt: u32) -> Option<u64> {
    if attempt >= 3 {
        None
    } else {
        Some(100 * (1_u64 << attempt))
    }
}
