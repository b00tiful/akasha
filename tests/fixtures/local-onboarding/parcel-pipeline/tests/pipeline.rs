use parcel_pipeline::{config::capacity, delivery::flush, queue::Queue};

#[test]
fn configuration_boundaries() {
    assert_eq!(capacity(None), Ok(2));
    for value in ["1", "16"] { assert!(capacity(Some(value)).is_ok()); }
    for value in ["0", "17", "bad", " 2"] { assert!(capacity(Some(value)).is_err()); }
    assert!(Queue::new(0).is_err());
    // Queue itself has no configuration-layer upper bound.
    assert!(Queue::new(17).is_ok());
}
#[test]
fn duplicate_precedes_full_check() {
    let mut queue = Queue::new(1).unwrap();
    assert_eq!(queue.submit("a"), Ok(true));
    assert_eq!(queue.submit("a"), Ok(false));
    assert_eq!(queue.submit("b"), Err("full"));
    assert_eq!(queue.len(), 1);
}
#[test]
fn failure_preserves_head_and_resume_order() {
    let mut queue = Queue::new(capacity(None).unwrap()).unwrap();
    queue.submit("a").unwrap(); queue.submit("b").unwrap();
    let mut seen = Vec::new();
    let report = flush(&mut queue, |id| { seen.push(id.to_owned()); id != "b" });
    assert_eq!(seen, ["a", "b"]);
    assert_eq!(report.render(), "sent=1 failed=true remaining=1");
    assert_eq!(queue.front(), Some("b"));
    assert_eq!(queue.submit("b"), Ok(false));
    assert_eq!(flush(&mut queue, |_| true).render(), "sent=1 failed=false remaining=0");
    assert!(queue.is_empty());
}
#[test]
fn empty_queue_does_not_send() {
    let mut queue = Queue::new(1).unwrap();
    let report = flush(&mut queue, |_| panic!("unexpected send"));
    assert_eq!(report.render(), "sent=0 failed=false remaining=0");
}
