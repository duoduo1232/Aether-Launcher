use super::nearest_release;

#[test]
fn maps_known_data_versions_to_releases() {
    assert_eq!(nearest_release(1343), Some("1.12.2"));
    assert_eq!(nearest_release(2586), Some("1.16.5"));
    assert_eq!(nearest_release(3954), Some("1.21"));
    assert_eq!(nearest_release(3955), Some("1.21.1"));
    assert_eq!(nearest_release(4189), Some("1.21.4"));
}

#[test]
fn unknown_versions_fall_back_to_the_latest_known_release() {
    assert_eq!(nearest_release(999_999), Some("1.21.8"));
    assert_eq!(nearest_release(4441), Some("1.21.8"));
}

#[test]
fn versions_older_than_the_table_have_no_release() {
    assert_eq!(nearest_release(0), None);
    assert_eq!(nearest_release(1342), None);
    assert_eq!(nearest_release(i32::MIN), None);
}
