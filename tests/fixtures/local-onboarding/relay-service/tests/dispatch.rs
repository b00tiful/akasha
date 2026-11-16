use relay_service::{dispatch::drain, queue::Queue, store::Store};
#[test]
fn invalid_head_stops_without_loss() {
    let mut q = Queue::new(3);
    let mut s = Store::default();
    q.push("valid".into());
    q.push("".into());
    q.push("later".into());
    let m = drain(&mut q, &mut s);
    assert_eq!(m.stored, 1);
    assert!(m.blocked);
    assert_eq!(m.remaining, 2);
    assert_eq!(q.front(), Some(""));
    assert_eq!(s.records, vec![b"valid".to_vec()]);
}
