use relay_service::codec::encode;
#[test]
fn payload_byte_boundaries() {
    assert!(encode("").is_none());
    assert!(encode(&"x".repeat(256)).is_some());
    assert!(encode(&"x".repeat(257)).is_none());
    assert_eq!(encode("世界"), Some("世界".as_bytes().to_vec()));
}
