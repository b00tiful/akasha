use relay_service::backoff::delay_ms;
#[test]
fn retry_boundaries() {
    assert_eq!(delay_ms(0), Some(100));
    assert_eq!(delay_ms(1), Some(200));
    assert_eq!(delay_ms(2), Some(400));
    assert_eq!(delay_ms(3), None);
    assert_eq!(delay_ms(u32::MAX), None);
}
