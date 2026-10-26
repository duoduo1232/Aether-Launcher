use super::schematics::validate_file_name;

#[test]
fn accepts_bare_litematic_file_names() {
    assert!(validate_file_name("house.litematic").is_ok());
    assert!(validate_file_name("测试投影.litematic").is_ok());
    assert!(validate_file_name("my-schematic_v2.litematic").is_ok());
}

#[test]
fn rejects_names_that_escape_the_managed_directory() {
    assert!(validate_file_name("schematic").is_err());
    assert!(validate_file_name(".litematic").is_err());
    assert!(validate_file_name("").is_err());
    assert!(validate_file_name("../evil.litematic").is_err());
    assert!(validate_file_name("a/b.litematic").is_err());
    assert!(validate_file_name("a\\b.litematic").is_err());
    assert!(validate_file_name("C:evil.litematic").is_err());
    assert!(validate_file_name("con.litematic").is_err());
    assert!(validate_file_name("trailing.litematic.").is_err());
    assert!(validate_file_name(&"x".repeat(256)).is_err());
}
