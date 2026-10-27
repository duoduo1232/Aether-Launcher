use std::fs;
use std::path::PathBuf;

use aether_core::{GameInstance, InstanceId, InstanceState, VersionId};

use super::{
    MAX_RELATIVE_DEPTH, MAX_SCAN_ENTRIES, collect_litematics, copy_one_schematic, scan_schematics,
    schematic_path, unique_destination, validate_folder_path, validate_relative_path,
};

#[test]
fn accepts_relative_litematic_paths() {
    assert!(validate_relative_path("house.litematic", true).is_ok());
    assert!(validate_relative_path("测试投影.litematic", true).is_ok());
    assert!(validate_relative_path("my-schematic_v2.litematic", true).is_ok());
    // 子文件夹结构要保留，所以带 `/` 的相对路径必须放行。
    assert!(validate_relative_path("城堡/主楼.litematic", true).is_ok());
    assert!(validate_relative_path("a/b/c.litematic", true).is_ok());
}

#[test]
fn rejects_paths_that_escape_the_managed_directory() {
    assert!(validate_relative_path("schematic", true).is_err());
    assert!(validate_relative_path(".litematic", true).is_err());
    assert!(validate_relative_path("", true).is_err());
    assert!(validate_relative_path("../evil.litematic", true).is_err());
    assert!(validate_relative_path("a/../../evil.litematic", true).is_err());
    assert!(validate_relative_path("/abs.litematic", true).is_err());
    assert!(validate_relative_path("a//b.litematic", true).is_err());
    assert!(validate_relative_path("a\\b.litematic", true).is_err());
    assert!(validate_relative_path("C:evil.litematic", true).is_err());
    assert!(validate_relative_path("con.litematic", true).is_err());
    assert!(validate_relative_path("trailing.litematic.", true).is_err());
    assert!(validate_relative_path(&"x".repeat(256), true).is_err());
    assert!(validate_relative_path(&format!("{}.litematic", "x".repeat(1024)), true).is_err());
}

#[test]
fn rejects_paths_nested_past_the_scan_limit() {
    let allowed = ["dir"; MAX_RELATIVE_DEPTH].join("/");
    assert!(validate_relative_path(&format!("{allowed}/deep.litematic"), true).is_ok());

    let too_deep = ["dir"; MAX_RELATIVE_DEPTH + 1].join("/");
    assert!(validate_relative_path(&format!("{too_deep}/deep.litematic"), true).is_err());
}

#[test]
fn folder_paths_allow_the_root_and_reject_traversal() {
    // 空字符串表示 `schematics/` 根目录。
    assert!(validate_folder_path("").is_ok());
    assert!(validate_folder_path("城堡").is_ok());
    assert!(validate_folder_path("城堡/主楼").is_ok());
    assert!(validate_folder_path("..").is_err());
    assert!(validate_folder_path("../outside").is_err());
    assert!(validate_folder_path("a\\b").is_err());
    assert!(validate_folder_path("C:/windows").is_err());
}

fn instance_with_root(root: PathBuf) -> GameInstance {
    GameInstance {
        id: InstanceId::new(),
        name: "测试实例".to_owned(),
        version: VersionId::parse("1.21.1").expect("version"),
        root,
        game_directory: None,
        state: InstanceState::Ready,
    }
}

fn temp_root(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!("aether-scan-{}-{label}", std::process::id()))
}

#[test]
fn scan_treats_missing_directory_as_no_schematics() {
    let (summaries, failed) = scan_schematics(&instance_with_root(temp_root("missing")));
    assert!(summaries.is_empty());
    assert!(failed.is_empty());
}

#[test]
fn scan_ignores_non_litematic_files_and_classifies_broken_ones() {
    let root = temp_root("content");
    let schematics = root.join("schematics");
    fs::create_dir_all(&schematics).expect("mkdir");
    fs::write(schematics.join("readme.txt"), b"not a schematic").expect("write txt");
    fs::write(schematics.join("broken.litematic"), b"definitely not gzip").expect("write bad");

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));
    assert!(summaries.is_empty());
    assert_eq!(failed.len(), 1);
    assert_eq!(failed[0].file_name, "broken.litematic");
    assert_eq!(failed[0].relative_path, "broken.litematic");
    assert_eq!(failed[0].reason, "unparseable");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scan_keeps_the_folder_structure_in_relative_paths() {
    let root = temp_root("nested");
    let schematics = root.join("schematics");
    // 用解析失败的文件验证遍历：`summarize_file` 的成败不影响相对路径的构造。
    fs::create_dir_all(schematics.join("城堡").join("主楼")).expect("mkdir nested");
    fs::create_dir_all(schematics.join("红石")).expect("mkdir sibling");
    fs::write(schematics.join("root.litematic"), b"broken").expect("write root");
    fs::write(schematics.join("城堡").join("broken.litematic"), b"broken").expect("write castle");
    fs::write(
        schematics.join("城堡").join("主楼").join("deep.litematic"),
        b"broken",
    )
    .expect("write deep");
    fs::write(schematics.join("红石").join("circuit.litematic"), b"broken")
        .expect("write redstone");
    // 子文件夹里的非投影文件同样要忽略。
    fs::write(schematics.join("城堡").join("notes.txt"), b"ignore me").expect("write txt");

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));
    assert!(summaries.is_empty());
    let paths: Vec<&str> = failed
        .iter()
        .map(|item| item.relative_path.as_str())
        .collect();
    assert_eq!(
        paths,
        vec![
            "root.litematic",
            "城堡/broken.litematic",
            "城堡/主楼/deep.litematic",
            "红石/circuit.litematic",
        ]
    );
    // 文件名只保留最后一段，路径信息在 `relative_path` 里。
    assert_eq!(failed[2].file_name, "deep.litematic");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scan_stops_at_the_depth_limit() {
    let root = temp_root("depth");
    let mut directory = root.join("schematics");
    for _ in 0..=MAX_RELATIVE_DEPTH {
        directory = directory.join("d");
    }
    fs::create_dir_all(&directory).expect("mkdir deep");
    fs::write(directory.join("too-deep.litematic"), b"broken").expect("write deep");

    let (_, failed) = scan_schematics(&instance_with_root(root.clone()));
    assert!(
        failed.is_empty(),
        "超过深度上限的文件不该被扫描：{failed:?}"
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn schematic_path_joins_relative_segments_under_schematics() {
    let instance = instance_with_root(PathBuf::from("instance-root"));
    assert_eq!(
        schematic_path(&instance, "castle.litematic"),
        PathBuf::from("instance-root")
            .join("schematics")
            .join("castle.litematic")
    );
    assert_eq!(
        schematic_path(&instance, "城堡/主楼/deep.litematic"),
        PathBuf::from("instance-root")
            .join("schematics")
            .join("城堡")
            .join("主楼")
            .join("deep.litematic")
    );
}

#[test]
fn unique_destination_renames_instead_of_overwriting() {
    let directory = temp_root("unique");
    fs::create_dir_all(&directory).expect("mkdir");
    fs::write(directory.join("castle.litematic"), b"first").expect("write");

    let picked = unique_destination(&directory, "castle.litematic").expect("pick");
    assert_eq!(
        picked.file_name().and_then(|name| name.to_str()),
        Some("castle (2).litematic")
    );

    fs::write(directory.join("castle (2).litematic"), b"second").expect("write");
    let picked = unique_destination(&directory, "castle.litematic").expect("pick");
    assert_eq!(
        picked.file_name().and_then(|name| name.to_str()),
        Some("castle (3).litematic")
    );

    // 没有冲突时保持原名。
    let picked = unique_destination(&directory, "fresh.litematic").expect("pick");
    assert_eq!(
        picked.file_name().and_then(|name| name.to_str()),
        Some("fresh.litematic")
    );

    fs::remove_dir_all(directory).expect("cleanup");
}

#[test]
fn collect_litematics_keeps_subfolders_and_skips_other_files() {
    let root = temp_root("collect");
    fs::create_dir_all(root.join("城堡/一层")).expect("mkdir");
    fs::write(root.join("a.litematic"), b"x").expect("write");
    fs::write(root.join("城堡/主楼.litematic"), b"x").expect("write");
    fs::write(root.join("城堡/一层/塔.litematic"), b"x").expect("write");
    fs::write(root.join("notes.txt"), b"x").expect("write");
    fs::write(root.join("城堡/readme.md"), b"x").expect("write");

    let mut files = Vec::new();
    let mut budget = MAX_SCAN_ENTRIES;
    collect_litematics(&root, "", 0, &mut files, &mut budget);
    files.sort();

    assert_eq!(
        files,
        vec![
            "a.litematic".to_owned(),
            "城堡/一层/塔.litematic".to_owned(),
            "城堡/主楼.litematic".to_owned(),
        ]
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn collect_litematics_stops_at_the_depth_limit() {
    let root = temp_root("collect-depth");
    let mut deep = root.clone();
    for index in 0..(MAX_RELATIVE_DEPTH + 2) {
        deep = deep.join(format!("d{index}"));
    }
    fs::create_dir_all(&deep).expect("mkdir");
    fs::write(deep.join("deep.litematic"), b"x").expect("write");

    let mut files = Vec::new();
    let mut budget = MAX_SCAN_ENTRIES;
    collect_litematics(&root, "", 0, &mut files, &mut budget);

    assert!(
        files.is_empty(),
        "past the depth limit nothing is collected"
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn copy_one_schematic_refuses_a_path_that_escapes() {
    let source = temp_root("copy-escape");
    let destination = temp_root("copy-escape-dest");
    fs::create_dir_all(&source).expect("mkdir");
    fs::create_dir_all(&destination).expect("mkdir");

    assert_eq!(
        copy_one_schematic(&source, &destination, "../evil.litematic"),
        Err("unsafe-relative-path")
    );

    fs::remove_dir_all(source).expect("cleanup");
    fs::remove_dir_all(destination).expect("cleanup");
}

#[test]
fn copy_one_schematic_rejects_a_file_that_is_not_a_litematic() {
    let source = temp_root("copy-broken");
    let destination = temp_root("copy-broken-dest");
    fs::create_dir_all(&source).expect("mkdir");
    fs::create_dir_all(&destination).expect("mkdir");
    fs::write(source.join("broken.litematic"), b"not gzip").expect("write");

    assert_eq!(
        copy_one_schematic(&source, &destination, "broken.litematic"),
        Err("unparseable")
    );
    assert!(
        !destination.join("broken.litematic").exists(),
        "a rejected file must not be published"
    );

    fs::remove_dir_all(source).expect("cleanup");
    fs::remove_dir_all(destination).expect("cleanup");
}
