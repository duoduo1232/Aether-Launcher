//! 投影图（`.litematic`，Litematica「投影」）的管理命令。
//!
//! 文件统一放在应用私有的 `schematics` 目录，目录路径由后端解析，IPC 只接受
//! 文件名（不含路径分隔符）。解析与大小校验在 `aether-schematic` 里完成；
//! 「复制到实例」的目标目录来自受信任的实例存储，不是前端给的路径。
//!
//! 导入走 `import_schematic`：前端读取用户选择的文件**内容**传进来，后端
//! 只按安全文件名落盘，不接触原始路径。

use std::fs;
use std::path::{Path, PathBuf};

use aether_core::InstanceId;
use aether_schematic::data_versions::nearest_release;
use aether_schematic::{LitematicMetadata, RegionData, parse, read_metadata};
use aether_storage::InstanceStore;
use serde::Serialize;
use tauri::Manager;

/// 单个投影文件的读取上限。
const MAX_SCHEMATIC_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// 投影图元数据（跨 IPC 的视图；附带数据版本对应的正式版名）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicMetadataDto {
    name: String,
    author: String,
    description: String,
    format_version: i32,
    minecraft_data_version: i32,
    created_for_release: Option<&'static str>,
    region_count: i32,
    total_blocks: i64,
    total_volume: i64,
    enclosing_size: [i32; 3],
}

impl SchematicMetadataDto {
    fn from_metadata(metadata: &LitematicMetadata) -> Self {
        Self {
            name: metadata.name.clone(),
            author: metadata.author.clone(),
            description: metadata.description.clone(),
            format_version: metadata.format_version,
            minecraft_data_version: metadata.minecraft_data_version,
            created_for_release: nearest_release(metadata.minecraft_data_version),
            region_count: metadata.region_count,
            total_blocks: metadata.total_blocks,
            total_volume: metadata.total_volume,
            enclosing_size: metadata.enclosing_size,
        }
    }
}

/// 投影图列表项。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicSummary {
    file_name: String,
    size_bytes: u64,
    metadata: SchematicMetadataDto,
}

/// 列表里解析失败的文件，按稳定分类上报，不回显文件路径。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedSchematic {
    file_name: String,
    reason: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicListing {
    schematics: Vec<SchematicSummary>,
    failed: Vec<FailedSchematic>,
}

/// 完整解码结果（预览数据）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicDetail {
    file_name: String,
    metadata: SchematicMetadataDto,
    regions: Vec<RegionData>,
}

/// 解析应用私有的投影图目录。
fn schematics_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("application data directory is unavailable: {error}"))?;
    let directory = base.join("schematics");
    fs::create_dir_all(&directory)
        .map_err(|_| "cannot prepare the schematics directory".to_owned())?;
    Ok(directory)
}

/// 实例存储根目录；与实例命令共用同一条规则：路径来自 Tauri，不来自 IPC。
fn instance_store_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("application data directory is unavailable: {error}"))?;
    fs::create_dir_all(base.join("instances"))
        .map_err(|error| format!("cannot prepare the instance store: {error}"))?;
    Ok(base)
}

/// IPC 只允许裸文件名：必须以 `.litematic` 结尾、不含任何路径分隔符、
/// 通过通用的相对路径安全检查（拒绝盘符、保留名、结尾点等）。
fn validate_file_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 255 {
        return Err("schematic file name must be 1-255 characters".to_owned());
    }
    if !name.ends_with(".litematic") || name == ".litematic" {
        return Err("schematic file name must end with .litematic".to_owned());
    }
    if name.contains(['/', '\\']) {
        return Err("schematic file name must not contain path separators".to_owned());
    }
    if !aether_download::is_safe_relative_path(Path::new(name)) {
        return Err("schematic file name is not a safe file name".to_owned());
    }
    Ok(())
}

fn read_capped(path: &Path) -> Result<Vec<u8>, String> {
    let size = fs::metadata(path)
        .map_err(|_| "cannot inspect schematic file".to_owned())?
        .len();
    if size > MAX_SCHEMATIC_FILE_BYTES {
        return Err("schematic file exceeds the 64 MiB limit".to_owned());
    }
    fs::read(path).map_err(|_| "cannot read schematic file".to_owned())
}

/// 列出受管理目录里的投影图；解析失败的文件按类别上报，不中断列表。
#[tauri::command]
pub(crate) async fn list_schematics(app: tauri::AppHandle) -> Result<SchematicListing, String> {
    let directory = schematics_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut schematics = Vec::new();
        let mut failed = Vec::new();
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(_) => return Ok(SchematicListing { schematics, failed }),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !file_name.ends_with(".litematic") {
                continue;
            }
            let reason = match describe_failure(&path) {
                Ok(summary) => {
                    schematics.push(summary);
                    continue;
                }
                Err(reason) => reason,
            };
            failed.push(FailedSchematic {
                file_name: file_name.to_owned(),
                reason,
            });
        }
        schematics.sort_by(|left, right| left.file_name.cmp(&right.file_name));
        Ok(SchematicListing { schematics, failed })
    })
    .await
    .map_err(|_| "schematic listing task failed".to_owned())?
}

fn describe_failure(path: &Path) -> Result<SchematicSummary, &'static str> {
    let size = fs::metadata(path).map_err(|_| "unreadable")?.len();
    if size > MAX_SCHEMATIC_FILE_BYTES {
        return Err("oversized");
    }
    let bytes = fs::read(path).map_err(|_| "unreadable")?;
    let metadata = read_metadata(&bytes).map_err(|_| "unparseable")?;
    Ok(SchematicSummary {
        file_name: path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_owned(),
        size_bytes: size,
        metadata: SchematicMetadataDto::from_metadata(&metadata),
    })
}

/// 读取并完整解码一个投影文件，返回预览所需的逐区域方块索引。
#[tauri::command]
pub(crate) async fn read_schematic(
    app: tauri::AppHandle,
    file_name: String,
) -> Result<SchematicDetail, String> {
    validate_file_name(&file_name)?;
    let directory = schematics_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = read_capped(&directory.join(&file_name))?;
        let schematic = parse(&bytes).map_err(|error| error.to_string())?;
        Ok(SchematicDetail {
            file_name,
            metadata: SchematicMetadataDto::from_metadata(&schematic.metadata),
            regions: schematic.regions,
        })
    })
    .await
    .map_err(|_| "schematic read task failed".to_owned())?
}

/// 导入投影文件内容到受管理目录；已存在的同名文件不会被覆盖。
#[tauri::command]
pub(crate) async fn import_schematic(
    app: tauri::AppHandle,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    validate_file_name(&file_name)?;
    if bytes.len() as u64 > MAX_SCHEMATIC_FILE_BYTES {
        return Err("schematic file exceeds the 64 MiB limit".to_owned());
    }
    let directory = schematics_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let destination = directory.join(&file_name);
        if destination.exists() {
            return Err("a schematic with this name already exists".to_owned());
        }
        // 先写临时文件再原子改名，避免半截文件被当作可解析的投影。
        let staging = directory.join(format!(".import-{}-{file_name}.tmp", std::process::id()));
        let result = (|| {
            fs::write(&staging, &bytes)
                .map_err(|_| "cannot write the staged schematic".to_owned())?;
            fs::rename(&staging, &destination)
                .map_err(|_| "cannot publish the schematic".to_owned())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&staging);
        }
        result?;
        // 导入后立即校验：宁可当场报错，也不留一个列表里显示「解析失败」的文件。
        read_metadata(&fs::read(&destination).map_err(|_| "cannot re-read the import")?)
            .map_err(|_| "imported file is not a valid litematic")?;
        Ok(file_name)
    })
    .await
    .map_err(|_| "schematic import task failed".to_owned())?
}

/// 删除受管理目录里的一个投影文件。
#[tauri::command]
pub(crate) async fn delete_schematic(
    app: tauri::AppHandle,
    file_name: String,
) -> Result<(), String> {
    validate_file_name(&file_name)?;
    let directory = schematics_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        match fs::remove_file(directory.join(&file_name)) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                Err("schematic not found".to_owned())
            }
            Err(_) => Err("cannot delete the schematic".to_owned()),
        }
    })
    .await
    .map_err(|_| "schematic delete task failed".to_owned())?
}

/// 把一个投影文件复制到指定实例的游戏目录 `schematics/` 下。
///
/// 实例目录来自受信任的实例存储；同名文件已存在时报错，不覆盖实例自己的
/// 投影。返回复制后的文件名。
#[tauri::command]
pub(crate) async fn copy_schematic_to_instance(
    app: tauri::AppHandle,
    file_name: String,
    instance_id: String,
) -> Result<String, String> {
    validate_file_name(&file_name)?;
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let store_root = instance_store_root(&app)?;
    let directory = schematics_dir(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store = InstanceStore::open(&store_root)
            .map_err(|_| "cannot open the instance store".to_owned())?;
        let instance = store
            .load(id)
            .map_err(|_| "instance not found in the managed store".to_owned())?;

        let target_dir = instance.root.join("schematics");
        fs::create_dir_all(&target_dir)
            .map_err(|_| "cannot prepare the instance schematics directory".to_owned())?;
        let destination = target_dir.join(&file_name);
        if destination.exists() {
            return Err("the instance already has a schematic with this name".to_owned());
        }
        let bytes = read_capped(&directory.join(&file_name))?;
        fs::write(&destination, &bytes)
            .map_err(|_| "cannot copy the schematic into the instance".to_owned())?;
        Ok(file_name)
    })
    .await
    .map_err(|_| "schematic copy task failed".to_owned())?
}

#[cfg(test)]
#[path = "schematics_tests.rs"]
mod tests;
