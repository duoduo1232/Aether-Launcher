//! 资源包缓存：把提取结果落盘，避免每次打开预览都重新解 25 MB 的 jar。
//!
//! 缓存键是 jar 的「路径 + 大小 + 修改时间」的哈希：jar 换了（更新版本、换
//! 整合包）就会自然失效，不需要手工清缓存。
//!
//! 缓存内容两份文件：
//! - `pack.json`：烘焙好的方块模型 + 图集 UV 槽（前端要的全部结构数据）；
//! - `atlas.png`：图集本体。
//!
//! 分开存是因为 JSON 很小、图集可能几百 KB，前端可以分别取，也便于排查。

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use super::{AssetPack, cache_dir};

/// 缓存里的元信息。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct CachedPack {
    source_version: String,
    blocks: std::collections::HashMap<String, Vec<super::VariantBinding>>,
    atlas: std::collections::HashMap<String, super::AtlasSlot>,
    atlas_size: u32,
}

/// 一个版本在磁盘上的 jar，供界面列出「可以从哪个版本提取」。
#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionJar {
    /// 版本目录名，界面显示用。
    pub version: String,
    /// jar 的绝对路径（只在后端使用，不序列化给前端）。
    #[serde(skip)]
    pub jar: PathBuf,
    /// jar 大小，界面用来判断「像不像完整的版本」。
    pub size_bytes: u64,
}

/// 列出一个 `.minecraft/versions` 下所有带 jar 的版本。
///
/// 只做目录枚举与 `metadata`，不读文件内容；按版本名排序保证界面稳定。
#[must_use]
pub fn list_version_jars(game_directory: &Path) -> Vec<VersionJar> {
    let versions = game_directory.join("versions");
    let Ok(entries) = fs::read_dir(&versions) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        if !entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false) {
            continue;
        }
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        // 优先取与版本目录同名的 jar（`<版本>/<版本>.jar`）—— 这才是客户端。
        // 版本目录里还可能有 loader 的 jar（Fabric/Forge 装完后常见），它们的
        // 大小也不小，随便取第一个可能拿到的是 loader，解出来没有方块模型。
        let expected = format!("{name}.jar");
        let files: Vec<_> = fs::read_dir(entry.path())
            .map(|dir| dir.flatten().collect::<Vec<_>>())
            .unwrap_or_default();

        let mut picked: Option<(PathBuf, u64)> = None;
        for file in files {
            let file_name = file.file_name();
            let Some(file_name) = file_name.to_str() else {
                continue;
            };
            if !file_name.ends_with(".jar") {
                continue;
            }
            let Ok(metadata) = file.metadata() else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            // 8 MB 以下的 jar 基本不是完整客户端，提取出来也没有方块模型。
            if metadata.len() < MIN_LIBRARY_JAR_BYTES {
                continue;
            }
            let exact = file_name == expected;
            let better = match &picked {
                Some(_) => exact,
                None => true,
            };
            if better {
                picked = Some((file.path(), metadata.len()));
            }
            if exact {
                break;
            }
        }

        if let Some((jar, size_bytes)) = picked {
            out.push(VersionJar {
                version: name.clone(),
                jar,
                size_bytes,
            });
        }
    }
    out.sort_by(|left, right| left.version.cmp(&right.version));
    out
}

/// 小于这个大小的 jar 不当作版本客户端（库 jar 通常几百 KB）。
const MIN_LIBRARY_JAR_BYTES: u64 = 8 * 1024 * 1024;

/// 一个版本 jar 的缓存键。
///
/// 用「路径 + 大小 + 修改时间」而不是文件内容哈希：后者要读完 25 MB，
/// 而这三项足以判断 jar 是否被替换。
#[must_use]
pub fn cache_key(jar: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    jar.to_string_lossy().hash(&mut hasher);
    if let Ok(metadata) = fs::metadata(jar) {
        metadata.len().hash(&mut hasher);
        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(std::time::UNIX_EPOCH)
        {
            duration.as_secs().hash(&mut hasher);
        }
    }
    format!("{:016x}", hasher.finish())
}

/// 读取缓存；不存在或损坏返回 `None`。
#[must_use]
pub fn load_cached(root: &Path, key: &str) -> Option<(AssetPack, Vec<u8>)> {
    let directory = cache_dir(root, key);
    let meta = fs::read_to_string(directory.join("pack.json")).ok()?;
    let cached: CachedPack = serde_json::from_str(&meta).ok()?;
    let atlas_png = fs::read(directory.join("atlas.png")).ok()?;
    let pack = AssetPack {
        source_version: cached.source_version,
        blocks: cached.blocks,
        atlas_png: Vec::new(),
        atlas: cached.atlas,
        atlas_size: cached.atlas_size,
    };
    Some((pack, atlas_png))
}

/// 写入缓存。失败不影响预览（下次重新提取即可），所以只需要布尔结果。
pub fn store_cached(root: &Path, key: &str, pack: &AssetPack) -> bool {
    let directory = cache_dir(root, key);
    if fs::create_dir_all(&directory).is_err() {
        return false;
    }
    let meta = CachedPack {
        source_version: pack.source_version.clone(),
        blocks: pack.blocks.clone(),
        atlas: pack.atlas.clone(),
        atlas_size: pack.atlas_size,
    };
    let Ok(json) = serde_json::to_string(&meta) else {
        return false;
    };
    fs::write(directory.join("pack.json"), json).is_ok()
        && fs::write(directory.join("atlas.png"), &pack.atlas_png).is_ok()
}
