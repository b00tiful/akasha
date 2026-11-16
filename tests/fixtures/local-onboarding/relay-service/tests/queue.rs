use relay_service::queue::Queue;
#[test]
fn deduplication_and_capacity() {
    let mut q = Queue::new(1);
    assert!(q.push("hello".into()));
    assert!(!q.push("hello".into()));
    assert!(!q.push("world".into()));
    q.ack();
    assert!(q.push("world".into()));
}
