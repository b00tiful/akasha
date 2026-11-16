use crate::{codec::encode, metrics::Metrics, queue::Queue, store::Store};
/// Drain stops at the first invalid head without acknowledging it; earlier valid heads are stored.
pub fn drain(queue: &mut Queue, store: &mut Store) -> Metrics {
    let mut result = Metrics::default();
    while let Some(head) = queue.front() {
        let Some(bytes) = encode(head) else {
            result.blocked = true;
            break;
        };
        store.append(bytes);
        queue.ack();
        result.stored += 1;
    }
    result.remaining = queue.len();
    result
}
