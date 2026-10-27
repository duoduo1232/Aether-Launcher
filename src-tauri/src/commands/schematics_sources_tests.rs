//! 多来源扫描与名字回退的测试。
//!
//! `.minecraft` 里投影是三处共用的：游戏目录根上的 `schematics/`、
//! `versions/<版本>/schematics`，以及实例自己的目录。只扫其中一处会出现
//! 「用户看得到文件、界面是空的」这种问题，所以这里把三处都钉住。

use std::fs;
use std::path::{Path, PathBuf};

use aether_core::{GameInstance, InstanceId, InstanceState, VersionId};

use super::{ORIGIN_GAME, ORIGIN_INSTANCE, ORIGIN_VERSION, display_name, scan_schematics};

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
    std::env::temp_dir().join(format!("aether-sources-{}-{label}", std::process::id()))
}

/// 写一个最小的合法 `.litematic`，元数据名字可控。
fn write_litematic(path: &Path, name: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("mkdir");
    }
    fs::write(path, litematic_bytes(name)).expect("write");
}

fn litematic_bytes(name: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    payload.push(10u8); // TAG_Compound 根
    write_name(&mut payload, "");
    // Version / MinecraftDataVersion 在根上
    payload.extend_from_slice(&[3, 0, 7]);
    payload.extend_from_slice(b"Version");
    payload.extend_from_slice(&6i32.to_be_bytes());
    payload.extend_from_slice(&[3, 0, 20]);
    payload.extend_from_slice(b"MinecraftDataVersion");
    payload.extend_from_slice(&3953i32.to_be_bytes());
    // Metadata 复合：名字、作者、描述、区域计数、总块数、总体积、包围盒
    payload.extend_from_slice(&[10, 0, 8]);
    payload.extend_from_slice(b"Metadata");
    nbt_string(&mut payload, "Name", name);
    nbt_string(&mut payload, "Author", "作者");
    nbt_string(&mut payload, "Description", "");
    nbt_int(&mut payload, "RegionCount", 1);
    nbt_int(&mut payload, "TotalBlocks", 1);
    nbt_int(&mut payload, "TotalVolume", 1);
    payload.extend_from_slice(&[10, 0, 13]);
    payload.extend_from_slice(b"EnclosingSize");
    nbt_int(&mut payload, "x", 1);
    nbt_int(&mut payload, "y", 1);
    nbt_int(&mut payload, "z", 1);
    payload.push(0); // 结束 EnclosingSize
    payload.push(0); // 结束 Metadata
    payload.push(0); // 结束根

    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    use std::io::Write;
    encoder.write_all(&payload).expect("gzip");
    encoder.finish().expect("gzip finish")
}

fn write_name(out: &mut Vec<u8>, name: &str) {
    let bytes = name.as_bytes();
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(bytes);
}

fn nbt_string(out: &mut Vec<u8>, key: &str, value: &str) {
    out.push(8); // TAG_String
    write_name(out, key);
    write_name(out, value);
}

fn nbt_int(out: &mut Vec<u8>, key: &str, value: i32) {
    out.push(3); // TAG_Int
    write_name(out, key);
    out.extend_from_slice(&value.to_be_bytes());
}

#[test]
fn scans_the_game_directory_when_the_instance_directory_is_empty() {
    let root = temp_root("game-root");
    // 实例自己的 schematics 不存在，只有游戏目录根上有。
    write_litematic(
        &root
            .join("minecraft")
            .join("schematics")
            .join("a.litematic"),
        "来自游戏目录",
    );

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));

    assert_eq!(summaries.len(), 1, "failed={failed:?}");
    assert_eq!(summaries[0].origin, ORIGIN_GAME);
    assert_eq!(summaries[0].origin_version, "");
    assert_eq!(summaries[0].relative_path, "a.litematic");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scans_every_version_schematics_directory() {
    let root = temp_root("versions");
    let game = root.join("minecraft");
    write_litematic(
        &game
            .join("versions")
            .join("1.20.1")
            .join("schematics")
            .join("one.litematic"),
        "一点二",
    );
    write_litematic(
        &game
            .join("versions")
            .join("1.21.4")
            .join("schematics")
            .join("two.litematic"),
        "一点二一",
    );
    // 没有 schematics 的版本目录不该造成任何影响。
    fs::create_dir_all(game.join("versions").join("共用一个库")).expect("mkdir");

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));

    assert_eq!(summaries.len(), 2, "failed={failed:?}");
    assert!(summaries.iter().all(|item| item.origin == ORIGIN_VERSION));
    let versions: Vec<&str> = summaries
        .iter()
        .map(|item| item.origin_version.as_str())
        .collect();
    assert_eq!(versions, vec!["1.20.1", "1.21.4"]);

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn keeps_the_same_relative_path_from_two_sources_apart() {
    let root = temp_root("dedupe");
    let game = root.join("minecraft");
    write_litematic(&root.join("schematics").join("a.litematic"), "实例的");
    write_litematic(
        &game
            .join("versions")
            .join("1.20.1")
            .join("schematics")
            .join("a.litematic"),
        "版本的",
    );

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));

    // 两个不同文件，同一条相对路径，两条都要留着。
    assert_eq!(summaries.len(), 2, "failed={failed:?}");
    assert_eq!(summaries[0].origin, ORIGIN_INSTANCE);
    assert_eq!(summaries[1].origin, ORIGIN_VERSION);
    assert_eq!(summaries[1].origin_version, "1.20.1");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn keeps_the_folder_structure_inside_a_version_directory() {
    let root = temp_root("version-folders");
    write_litematic(
        &root
            .join("minecraft")
            .join("versions")
            .join("生电-1.21.4")
            .join("schematics")
            .join("建筑")
            .join("空城计.litematic"),
        "Unnamed",
    );

    let (summaries, failed) = scan_schematics(&instance_with_root(root.clone()));

    assert_eq!(summaries.len(), 1, "failed={failed:?}");
    assert_eq!(summaries[0].relative_path, "建筑/空城计.litematic");
    // 名字是占位值时用文件名，文件名去掉扩展名。
    assert_eq!(summaries[0].metadata.name, "空城计");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn display_name_falls_back_when_the_metadata_name_is_a_placeholder() {
    assert_eq!(display_name("Unnamed", "空城计.litematic"), "空城计");
    assert_eq!(display_name("unnamed", "a.litematic"), "a");
    assert_eq!(display_name("  ", "b.litematic"), "b");
    assert_eq!(display_name("", "c.litematic"), "c");
    // 真填了名字就尊重它，即使和文件名不同。
    assert_eq!(display_name("我的城堡", "castle.litematic"), "我的城堡");
    // 极端情况：没有文件名可用时退回原值，不产生空标题。
    assert_eq!(display_name("Unnamed", ".litematic"), "Unnamed");
}
