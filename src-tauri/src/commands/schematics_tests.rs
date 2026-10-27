use std::fs;
use std::path::PathBuf;

use aether_core::{GameInstance, InstanceId, InstanceState, VersionId};

use super::{scan_schematics, validate_file_name};

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

fn instance_with_root(root: PathBuf) -> GameInstance {
    GameInstance {
        id: InstanceId::new(),
        name: "测试实例".to_owned(),
        version: VersionId::parse("1.21.1").expect("version"),
        root,
        state: InstanceState::Ready,
    }
}

#[test]
fn scan_treats_missing_directory_as_no_schematics() {
    let temp = std::env::temp_dir().join(format!("aether-scan-{}-missing", std::process::id()));
    let (summaries, failed) = scan_schematics(&instance_with_root(temp));
    assert!(summaries.is_empty());
    assert!(failed.is_empty());
}

#[test]
fn scan_ignores_non_litematic_files_and_classifies_broken_ones() {
    let root = std::env::temp_dir().join(format!("aether-scan-{}-content", std::process::id()));
    let schematics = root.join("schematics");
    fs::create_dir_all(&schematics).expect("mkdir");
    fs::write(schematics.join("readme.txt"), b"not a schematic").expect("write txt");
    fs::write(schematics.join("broken.litematic"), b"definitely not gzip").expect("write bad");

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));
    assert!(summaries.is_empty());
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].file_name, "broken.litematic");
    assert_eq!(failed[0].reason, "unparseable");

    fs::remove_dir_all(root).expect("cleanup");
}
