//! 找出一个实例真正使用的 `.minecraft`。
//!
//! 老记录（`game_directory` 字段加入之前创建的实例）里没有游戏目录，只留下一个
//! 空的 `minecraft/`。这些实例的游戏其实在用户的 `.minecraft` 里，所以读取时
//! 需要反查一次。
//!
//! # 为什么按版本名反查
//!
//! 猜路径是不行的：机器上可能有好几个 `.minecraft`，随便挑一个会把实例指到
//! 别人的游戏上。这里要求候选目录的 `versions/<版本>/` **确实存在**，也就是
//! 「这个目录里有这个版本」。用户机器上 134 个实例的版本名全部能在真实
//! `.minecraft` 里对上，所以这个判据能精确落到正确目录。
//!
//! # 边界
//!
//! - 只读，绝不写入调用方给的记录；
//! - 候选目录数量有上限，`versions/` 只做存在性检查，不遍历内容；
//! - 符号链接一律拒绝，防止被指向别处。

use std::path::{Path, PathBuf};

use crate::import::pcl2;

/// 一次反查最多考虑多少个候选目录。
const MAX_CANDIDATES: usize = 16;

/// 已知启动器在用户目录下的常见位置（相对 `%APPDATA%` / 用户目录）。
///
/// 这些只是「哪个目录值得检查」的提示，不构成信任：真正决定的是
/// `versions/<版本>/` 是否存在。
const COMMON_GAME_DIRECTORIES: &[&str] = &[
    ".minecraft",
    "PCL/.minecraft",
    "AppData/Roaming/.minecraft",
    "Desktop/.minecraft",
    "Documents/.minecraft",
];

/// 找一个包含 `version` 这个版本的游戏目录。
///
/// `hints` 是优先检查的目录（比如启动器配置里记录的路径），`search_roots` 是其
/// 后备搜索起点。返回第一个通过校验的目录；都不通过时返回 `None`。
///
/// 校验只有一条：`<候选>/versions/<version>/` 是一个真实目录。
#[must_use]
pub fn locate_game_directory_for_version(
    version: &str,
    hints: &[PathBuf],
    search_roots: &[PathBuf],
) -> Option<PathBuf> {
    if !is_usable_version_name(version) {
        return None;
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    for hint in hints {
        push_unique(&mut candidates, hint.clone());
    }
    for root in search_roots {
        for relative in common_relatives(root) {
            push_unique(&mut candidates, relative);
        }
    }

    candidates
        .into_iter()
        .take(MAX_CANDIDATES)
        .find(|candidate| has_version(candidate, version))
}

/// 一个游戏目录里是否真有这个版本。
///
/// 只看 `versions/<version>/` 是否存在且是真实目录；不读里面的 JSON，
/// 也不跟随符号链接。
#[must_use]
pub fn has_version(game_directory: &Path, version: &str) -> bool {
    if !is_usable_version_name(version) {
        return false;
    }
    let Ok(metadata) = std::fs::symlink_metadata(game_directory) else {
        return false;
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return false;
    }
    let version_directory = game_directory.join("versions").join(version);
    match std::fs::symlink_metadata(&version_directory) {
        Ok(metadata) => metadata.is_dir() && !metadata.file_type().is_symlink(),
        Err(_) => false,
    }
}

/// 版本名必须能安全地当作一层目录名使用。
fn is_usable_version_name(version: &str) -> bool {
    !version.is_empty()
        && version.len() <= 255
        && !version.contains(['/', '\\'])
        && version != "."
        && version != ".."
}

/// PCL2 / HMCL / BakaXL 配置里记录的游戏目录。
///
/// 读出所有能找到的配置，把其中记录的游戏目录按顺序返回。读不到就是空列表，
/// 调用方继续用后备搜索。
#[must_use]
pub fn configured_game_directories(launcher_roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for root in launcher_roots {
        for candidate in pcl2::configured_game_directories_for_launcher(root) {
            push_unique(&mut directories, candidate);
        }
    }
    directories
}

/// 常见相对路径：`root/.minecraft`、`root/PCL/.minecraft` 等等。
fn common_relatives(root: &Path) -> Vec<PathBuf> {
    COMMON_GAME_DIRECTORIES
        .iter()
        .map(|relative| {
            relative
                .split('/')
                .fold(root.to_path_buf(), |path, segment| path.join(segment))
        })
        .collect()
}

fn push_unique(directories: &mut Vec<PathBuf>, path: PathBuf) {
    let key = path.to_string_lossy().to_ascii_lowercase();
    if directories
        .iter()
        .any(|existing| existing.to_string_lossy().to_ascii_lowercase() == key)
    {
        return;
    }
    directories.push(path);
}

#[cfg(test)]
#[path = "fallback_tests.rs"]
mod tests;
