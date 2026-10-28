//! 多来源扫描与名字回退的测试。
//!
//! `.minecraft` 里投影是三处共用的：游戏目录根上的 `schematics/`、
//! `versions/<版本>/schematics`，以及实例自己的目录。只扫其中一处会出现
//! 「用户看得到文件、界面是空的」这种问题，所以这里把三处都钉住。

use std::fs;
use std::path::{Path, PathBuf};

use aether_core::{GameInstance, InstanceId, InstanceState, VersionId};

use super::{
    FailedSchematic, ORIGIN_GAME, ORIGIN_INSTANCE, ORIGIN_VERSION, SchematicSummary, display_name,
    scan_directory, scan_sources,
};

/// 按实例在当前磁盘上的来源跑一次扫描，返回拆分后的两个列表。
///
/// 复刻 `list_schematics` 的拼接逻辑（按来源目录逐个扫描再合并），但不带
/// 跨实例缓存 —— 测试关心的是「给定磁盘状态，这个实例看到什么」。
fn scan_instance(instance: &GameInstance) -> (Vec<SchematicSummary>, Vec<FailedSchematic>) {
    let mut result = super::ScanResult::default();
    for source in &scan_sources(instance) {
        result.absorb(&scan_directory(source), source);
    }
    result.sort();
    (result.schematics, result.failed)
}

fn instance_with_root(root: PathBuf) -> GameInstance {
    instance_with_version(root, "1.21.1")
}

/// 指定版本号的实例；`versions/<version>/schematics` 只该由它看到。
fn instance_with_version(root: PathBuf, version: &str) -> GameInstance {
    GameInstance {
        id: InstanceId::new(),
        name: format!("测试实例 {version}"),
        version: VersionId::parse(version).expect("version"),
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

    let (summaries, failed) = scan_instance(&instance_with_root(root.clone()));

    assert_eq!(summaries.len(), 1, "failed={failed:?}");
    assert_eq!(summaries[0].origin, ORIGIN_GAME);
    assert_eq!(summaries[0].origin_version, "");
    assert_eq!(summaries[0].relative_path, "a.litematic");

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn scans_the_instance_own_version_directory() {
    // 投影跟着版本走：`versions/1.20.1/schematics` 只属于 1.20.1 的实例，
    // 别的版本看不到（否则每个版本都会显示整个 `.minecraft` 的投影总和）。
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

    let mut instance = instance_with_version(root.clone(), "1.20.1");
    instance.game_directory = Some(game.clone());
    let (summaries, failed) = scan_instance(&instance);

    assert_eq!(summaries.len(), 1, "failed={failed:?}");
    assert_eq!(summaries[0].origin, ORIGIN_VERSION);
    assert_eq!(summaries[0].origin_version, "1.20.1");
    assert_eq!(summaries[0].file_name, "one.litematic");

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

    let mut instance = instance_with_version(root.clone(), "1.20.1");
    instance.game_directory = Some(game.clone());
    let (summaries, failed) = scan_instance(&instance);

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

    let mut instance = instance_with_version(root.clone(), "生电-1.21.4");
    instance.game_directory = Some(root.join("minecraft"));
    let (summaries, failed) = scan_instance(&instance);
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

#[test]
fn a_version_only_sees_its_own_version_directory() {
    // 回归测试，两次踩坑都钉在这里：
    //
    // 1. 曾把「扫描结果」按游戏目录缓存，共用同一个 `.minecraft` 的实例全部
    //    显示同一份上千个投影；
    // 2. 曾把 `versions/` 下**所有**版本的 `schematics` 都算进每个实例，于是
    //    每个版本都显示整个 `.minecraft` 的投影总和 —— 用户看到的
    //    「每个版本都有 1000 多个」。
    //
    // 正确范围：实例私有目录 + 游戏目录根上的 `schematics/` + **自己那个版本**
    // 的 `versions/<版本>/schematics`。
    let root = temp_root("own-version");
    let game = root.join("game");
    fs::create_dir_all(game.join("versions").join("a").join("schematics")).expect("a");
    fs::create_dir_all(game.join("versions").join("b").join("schematics")).expect("b");
    fs::create_dir_all(game.join("schematics")).expect("shared");

    write_litematic(&game.join("schematics").join("公共.litematic"), "公共");
    write_litematic(
        &game
            .join("versions")
            .join("a")
            .join("schematics")
            .join("甲的.litematic"),
        "甲的",
    );
    write_litematic(
        &game
            .join("versions")
            .join("b")
            .join("schematics")
            .join("乙的.litematic"),
        "乙的",
    );

    let mut instance_a = instance_with_version(root.join("instance-a"), "a");
    instance_a.game_directory = Some(game.clone());
    let mut instance_b = instance_with_version(root.join("instance-b"), "b");
    instance_b.game_directory = Some(game.clone());

    let names = |items: &[SchematicSummary]| {
        let mut names: Vec<String> = items.iter().map(|item| item.file_name.clone()).collect();
        names.sort();
        names
    };

    let (a, _) = scan_instance(&instance_a);
    let (b, _) = scan_instance(&instance_b);

    // 公共的看得到，别人版本目录里的看不到。按码点排序，不做本地化折叠：
    // 公（U+516C）在 甲（U+7532）前面，乙（U+4E59）在 公 前面。
    assert_eq!(
        names(&a),
        vec!["公共.litematic".to_owned(), "甲的.litematic".to_owned()]
    );
    assert_eq!(
        names(&b),
        vec!["乙的.litematic".to_owned(), "公共.litematic".to_owned()]
    );

    // 实例私有的 schematics 只属于自己。
    write_litematic(
        &root
            .join("instance-a")
            .join("schematics")
            .join("只给甲.litematic"),
        "只给甲",
    );
    let (a, _) = scan_instance(&instance_a);
    let (b, _) = scan_instance(&instance_b);
    assert!(names(&a).contains(&"只给甲.litematic".to_owned()));
    assert!(!names(&b).contains(&"只给甲.litematic".to_owned()));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_directory_scan_does_not_carry_the_instances_origin_labels() {
    // 目录层缓存可以跨实例复用，但 `origin` / `originVersion` 描述的是
    // 「这个目录属于哪个实例」，必须由 `absorb` 逐条重新套用，不能沿用缓存。
    let root = temp_root("absorb");
    let game = root.join("game");
    fs::create_dir_all(game.join("schematics")).expect("shared");
    write_litematic(&game.join("schematics").join("公共.litematic"), "公共");

    let mut instance = instance_with_root(root.join("instance"));
    instance.game_directory = Some(game.clone());
    let (summaries, _) = scan_instance(&instance);
    let item = summaries
        .iter()
        .find(|item| item.file_name == "公共.litematic")
        .expect("schematic");
    assert_eq!(item.origin, ORIGIN_GAME);
    assert_eq!(item.origin_version, "");

    fs::remove_dir_all(root).expect("cleanup");
}
