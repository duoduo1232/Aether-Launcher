//! 资源包相关的 IPC 命令：列出可提取的版本、提取、读取已提取的结果。
//!
//! 提取是**用户主动触发**的：界面在预览里发现没有资源包时提示「选择版本提取」，
//! 用户点了才解 jar。这样不会在打开页面时悄悄解几十 MB 的文件。

use std::path::Path;

use aether_storage::list_instances;

use super::asset_pack::{
    VersionJar, build_asset_pack, cache_key, list_version_jars, load_cached, store_cached,
};
use super::{GameInstance, instance_store_root, resolve_game_directory};

/// 界面上「可以提取资源包」的一个候选版本。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetPackCandidate {
    /// 版本名（界面显示）。
    version: String,
    /// jar 大小（MB 取整），界面用来提示「这是完整客户端」。
    size_mb: u64,
    /// 是否已经从它提取过。
    extracted: bool,
}

/// 列出所有实例游戏目录里可提取的版本 jar，按版本名去重。
///
/// 只枚举目录和 `metadata`，不读 jar 内容，所以即使有上百个版本也是毫秒级。
#[tauri::command]
pub(crate) async fn list_asset_pack_candidates(
    app: tauri::AppHandle,
) -> Result<Vec<AssetPackCandidate>, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing =
            list_instances(&root).map_err(|error| format!("cannot read the store: {error}"))?;

        let mut seen = std::collections::HashSet::new();
        let mut candidates = Vec::new();
        for instance in &listing.instances {
            let game = resolve_game_directory(instance);
            for jar in list_version_jars(&game) {
                if !seen.insert(jar.version.clone()) {
                    continue;
                }
                let extracted = load_cached(&root, &cache_key(&jar.jar)).is_some();
                candidates.push(AssetPackCandidate {
                    version: jar.version,
                    size_mb: jar.size_bytes / (1024 * 1024),
                    extracted,
                });
            }
        }
        candidates.sort_by(|left, right| left.version.cmp(&right.version));
        Ok(candidates)
    })
    .await
    .map_err(|error| format!("asset pack listing task failed: {error}"))?
}

/// 提取结果：给前端的资源包 + 图集（PNG 的 data URL）。
///
/// 图集以 data URL 返回，前端直接 `TextureLoader.load` 即可，不需要额外的
/// 资源协议或临时文件 —— 只读一次，且内容很小（一张 512² 的图约几十 KB）。
#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssetPackDto {
    source_version: String,
    blocks: std::collections::HashMap<String, Vec<super::asset_pack::VariantBinding>>,
    atlas: std::collections::HashMap<String, super::asset_pack::AtlasSlot>,
    atlas_size: u32,
    atlas_data_url: String,
}

/// 从指定版本的 jar 提取（或读缓存）资源包。
///
/// `version` 必须是 [`list_asset_pack_candidates`] 里出现过的名字；后端按实例
/// 记录找到它所在游戏目录里的 jar，前端从不传路径。
#[tauri::command]
pub(crate) async fn load_asset_pack(
    app: tauri::AppHandle,
    version: String,
) -> Result<AssetPackDto, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing =
            list_instances(&root).map_err(|error| format!("cannot read the store: {error}"))?;

        let jar = listing
            .instances
            .iter()
            .flat_map(|instance| list_version_jars(&resolve_game_directory(instance)))
            .find(|candidate| candidate.version == version)
            .map(|candidate: VersionJar| candidate.jar)
            .ok_or_else(|| format!("version {version} has no usable client jar"))?;

        let key = cache_key(&jar);
        let (pack, atlas_png) = match load_cached(&root, &key) {
            Some(cached) => cached,
            None => {
                let pack = build_asset_pack(&jar, &version)
                    .map_err(|error| format!("cannot read the asset pack: {error}"))?;
                let atlas = pack.atlas_png.clone();
                let _ = store_cached(&root, &key, &pack);
                (pack, atlas)
            }
        };

        Ok(AssetPackDto {
            source_version: pack.source_version,
            blocks: pack.blocks,
            atlas: pack.atlas,
            atlas_size: pack.atlas_size,
            atlas_data_url: png_data_url(&atlas_png),
        })
    })
    .await
    .map_err(|error| format!("asset pack task failed: {error}"))?
}

/// 把 PNG 字节包成 data URL。
fn png_data_url(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 4 / 3 + 32);
    out.push_str("data:image/png;base64,");
    // 手写 base64：只为这一处引入一个 crate 不值得。
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(*chunk.get(1).unwrap_or(&0));
        let b2 = u32::from(*chunk.get(2).unwrap_or(&0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        let _ = write!(out, "{}", TABLE[(triple >> 18) as usize & 0x3f] as char);
        let _ = write!(out, "{}", TABLE[(triple >> 12) as usize & 0x3f] as char);
        if chunk.len() > 1 {
            let _ = write!(out, "{}", TABLE[(triple >> 6) as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            let _ = write!(out, "{}", TABLE[triple as usize & 0x3f] as char);
        } else {
            out.push('=');
        }
    }
    out
}

/// 预览窗口里是否已经有可用资源包（用于决定要不要提示用户提取）。
#[tauri::command]
pub(crate) async fn asset_pack_status(app: tauri::AppHandle) -> Result<bool, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing =
            list_instances(&root).map_err(|error| format!("cannot read the store: {error}"))?;
        let any = listing.instances.iter().any(|instance: &GameInstance| {
            list_version_jars(&resolve_game_directory(instance))
                .iter()
                .any(|candidate| load_cached(&root, &cache_key(&candidate.jar)).is_some())
        });
        Ok(any)
    })
    .await
    .map_err(|error| format!("asset pack status task failed: {error}"))?
}

/// 判断一个路径是不是文件（内部辅助，保留给测试）。
#[allow(dead_code)]
fn is_file(path: &Path) -> bool {
    path.is_file()
}
