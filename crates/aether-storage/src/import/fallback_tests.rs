//! 游戏目录回退查找的测试。
//!
//! 老实例没有 `game_directory` 字段，只能按版本名反查。这里钉住两件事：
//! 反查必须落到**真的包含该版本**的目录，且绝不因为路径可疑而乱指。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use super::{configured_game_directories, has_version, locate_game_directory_for_version};

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// 只有一个起点的搜索列表。
fn roots(path: &Path) -> Vec<PathBuf> {
    vec![path.to_path_buf()]
}

/// 创建目录符号链接；没有 `SeCreateSymbolicLinkPrivilege` 或开发者模式时返回 false，
/// 调用方跳过相关断言而不是失败。
#[cfg(windows)]
fn symlink_dir(original: &Path, link: &Path) -> bool {
    std::os::windows::fs::symlink_dir(original, link).is_ok()
}

#[cfg(not(windows))]
fn symlink_dir(original: &Path, link: &Path) -> bool {
    std::os::unix::fs::symlink(original, link).is_ok()
}

fn temp_root(label: &str) -> PathBuf {
    let unique = COUNTER.fetch_add(1, Ordering::SeqCst);
    let path = std::env::temp_dir().join(format!(
        "aether-fallback-{label}-{}-{unique}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("temp root");
    path
}

/// 造一个包含 `versions` 里指定版本的 `.minecraft`。
fn game_with_versions(root: &Path, versions: &[&str]) -> PathBuf {
    let root = root.to_path_buf();
    let game = root.join(".minecraft");
    for version in versions {
        fs::create_dir_all(game.join("versions").join(version)).expect("version dir");
    }
    game
}

#[test]
fn finds_the_directory_that_actually_holds_the_version() {
    let root = temp_root("find");
    let game = game_with_versions(&root, &["1.20.1"]);

    let found = locate_game_directory_for_version("1.20.1", &Vec::new(), &roots(&root));

    assert_eq!(found.as_deref(), Some(game.as_path()));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn does_not_pick_a_directory_that_lacks_the_version() {
    let root = temp_root("wrong");
    // 目录存在，但没有这个版本：不能认。
    game_with_versions(&root, &["1.21.4"]);

    assert!(locate_game_directory_for_version("1.20.1", &Vec::new(), &roots(&root)).is_none());

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn prefers_an_explicit_hint_over_the_common_locations() {
    let root = temp_root("hint");
    let elsewhere = root.join("custom-game");
    fs::create_dir_all(elsewhere.join("versions").join("1.20.1")).expect("hint version");
    // 常见位置里也有这个版本，但提示优先。
    game_with_versions(&root, &["1.20.1"]);

    let found = locate_game_directory_for_version(
        "1.20.1",
        &roots(&elsewhere),
        &roots(&root),
    );

    assert_eq!(found.as_deref(), Some(elsewhere.as_path()));

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn rejects_version_names_that_are_not_a_single_segment() {
    let root = temp_root("bad-version");
    game_with_versions(&root, &["1.20.1"]);

    for version in ["", "..", ".", "../1.20.1", "a/b", "a\\b"] {
        assert!(
            locate_game_directory_for_version(version, &Vec::new(), &roots(&root)).is_none(),
            "{version:?} must not resolve to a game directory"
        );
        assert!(!has_version(&root.join(".minecraft"), version));
    }

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_symlinked_game_directory_is_not_accepted() {
    let root = temp_root("symlink");
    let real = game_with_versions(&root, &["1.20.1"]);
    let link = root.join("linked");
    if !symlink_dir(&real, &link) {
        // 没有创建符号链接的权限时跳过：这条断言只用于拦住跟随链接的实现。
        fs::remove_dir_all(root).expect("cleanup");
        return;
    }

    assert!(
        !has_version(&link, "1.20.1"),
        "a symlinked game directory must be rejected"
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn pcl2_configuration_contributes_the_recorded_game_directory() {
    let root = temp_root("pcl2");
    let launcher = root.join("PCL");
    fs::create_dir_all(&launcher).expect("launcher");
    let game = game_with_versions(&root, &["1.20.1"]);
    fs::write(
        launcher.join("Setup.ini"),
        format!("LaunchFolderSelect:{}\n", game.display()),
    )
    .expect("setup.ini");

    let configured = configured_game_directories(&roots(&root));

    assert!(
        configured.iter().any(|candidate| candidate == &game),
        "the recorded game directory should be offered, got {configured:?}"
    );

    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn a_missing_launcher_configuration_offers_nothing() {
    let root = temp_root("no-config");
    assert!(configured_game_directories(&roots(&root)).is_empty());
    fs::remove_dir_all(root).expect("cleanup");
}
