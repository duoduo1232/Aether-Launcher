//! 投影图（`.litematic`，Litematica「投影」）命令：自动识别实例里的投影。
//!
//! 投影文件跟实例走：每个实例游戏目录下的 `schematics/` 都会被自动扫描。
//! IPC 只传实例 ID 和裸文件名（不含路径分隔符）；实例目录来自受信任的实例
//! 存储，前端从不给路径。导入走 `import_schematic`：前端读取用户选择的文件
//! **内容**传进来，后端按安全文件名写进指定实例；跨版本复制在实例之间进行。

use std::fs;
use std::path::{Path, PathBuf};

use aether_core::{GameInstance, InstanceId, InstanceState};
use aether_schematic::data_versions::nearest_release;
use aether_schematic::{LitematicMetadata, RegionData, parse, read_metadata};
use aether_storage::{InstanceStore, list_instances};
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

/// 一个实例里的投影图列表项。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicSummary {
    file_name: String,
    size_bytes: u64,
    metadata: SchematicMetadataDto,
}

/// 扫描时解析失败的文件，按稳定分类上报，不回显文件路径。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedSchematic {
    file_name: String,
    reason: &'static str,
}

/// 一个实例自动识别出的全部投影图。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSchematics {
    id: String,
    name: String,
    version: String,
    state: &'static str,
    schematics: Vec<SchematicSummary>,
    failed: Vec<FailedSchematic>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicListing {
    instances: Vec<InstanceSchematics>,
    skipped: Vec<String>,
}

/// 完整解码结果（预览数据）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicDetail {
    instance_id: String,
    file_name: String,
    metadata: SchematicMetadataDto,
    regions: Vec<RegionData>,
}

/// 实例存储根目录；路径来自 Tauri，不来自 IPC。
fn instance_store_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("application data directory is unavailable: {error}"))?;
    fs::create_dir_all(base.join("instances"))
        .map_err(|error| format!("cannot prepare the instance store: {error}"))?;
    Ok(base)
}

fn state_label(state: &InstanceState) -> &'static str {
    match state {
        InstanceState::Ready => "ready",
        InstanceState::Incomplete => "incomplete",
        InstanceState::Running => "running",
        InstanceState::Failed => "failed",
    }
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

/// 扫描一个实例的 `schematics/` 目录。目录不存在视为没有投影；解析失败的
/// 文件按类别归入 `failed`，不中断扫描。
fn scan_schematics(instance: &GameInstance) -> (Vec<SchematicSummary>, Vec<FailedSchematic>) {
    let mut summaries = Vec::new();
    let mut failed = Vec::new();
    let directory = instance.root.join("schematics");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(_) => return (summaries, failed),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        if !file_name.ends_with(".litematic") {
            continue;
        }
        match summarize_file(&path, file_name) {
            Ok(summary) => summaries.push(summary),
            Err(reason) => failed.push(FailedSchematic {
                file_name: file_name.to_owned(),
                reason,
            }),
        }
    }
    summaries.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    (summaries, failed)
}

fn summarize_file(path: &Path, file_name: &str) -> Result<SchematicSummary, &'static str> {
    let size = fs::metadata(path).map_err(|_| "unreadable")?.len();
    if size > MAX_SCHEMATIC_FILE_BYTES {
        return Err("oversized");
    }
    let bytes = fs::read(path).map_err(|_| "unreadable")?;
    let metadata = read_metadata(&bytes).map_err(|_| "unparseable")?;
    Ok(SchematicSummary {
        file_name: file_name.to_owned(),
        size_bytes: size,
        metadata: SchematicMetadataDto::from_metadata(&metadata),
    })
}

/// 自动识别每个实例游戏目录 `schematics/` 下的投影文件，按实例分组返回。
#[tauri::command]
pub(crate) async fn list_schematics(app: tauri::AppHandle) -> Result<SchematicListing, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing = list_instances(&root)
            .map_err(|error| format!("cannot scan the instance store: {error}"))?;
        let mut instances = Vec::new();
        for instance in &listing.instances {
            let (schematics, failed) = scan_schematics(instance);
            instances.push(InstanceSchematics {
                id: instance.id.to_string(),
                name: instance.name.clone(),
                version: instance.version.as_str().to_owned(),
                state: state_label(&instance.state),
                schematics,
                failed,
            });
        }
        Ok(SchematicListing {
            instances,
            skipped: listing.skipped.iter().map(|s| s.id.clone()).collect(),
        })
    })
    .await
    .map_err(|_| "schematic listing task failed".to_owned())?
}

/// 读取并完整解码指定实例里的一个投影文件，返回预览数据。
#[tauri::command]
pub(crate) async fn read_schematic(
    app: tauri::AppHandle,
    instance_id: String,
    file_name: String,
) -> Result<SchematicDetail, String> {
    validate_file_name(&file_name)?;
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let instance = store
            .load(id)
            .map_err(|_| "instance not found in the managed store".to_owned())?;
        let bytes = read_capped(&instance.root.join("schematics").join(&file_name))?;
        let schematic = parse(&bytes).map_err(|error| error.to_string())?;
        Ok(SchematicDetail {
            instance_id,
            file_name,
            metadata: SchematicMetadataDto::from_metadata(&schematic.metadata),
            regions: schematic.regions,
        })
    })
    .await
    .map_err(|_| "schematic read task failed".to_owned())?
}

/// 把文件内容写进指定实例的 `schematics/` 目录；同名文件已存在时拒绝覆盖。
#[tauri::command]
pub(crate) async fn import_schematic(
    app: tauri::AppHandle,
    instance_id: String,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    validate_file_name(&file_name)?;
    if bytes.len() as u64 > MAX_SCHEMATIC_FILE_BYTES {
        return Err("schematic file exceeds the 64 MiB limit".to_owned());
    }
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let instance = store
            .load(id)
            .map_err(|_| "instance not found in the managed store".to_owned())?;
        let directory = instance.root.join("schematics");
        fs::create_dir_all(&directory)
            .map_err(|_| "cannot prepare the instance schematics directory".to_owned())?;
        let destination = directory.join(&file_name);
        if destination.exists() {
            return Err("this instance already has a schematic with this name".to_owned());
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

/// 删除指定实例里的一个投影文件。
#[tauri::command]
pub(crate) async fn delete_schematic(
    app: tauri::AppHandle,
    instance_id: String,
    file_name: String,
) -> Result<(), String> {
    validate_file_name(&file_name)?;
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let instance = store
            .load(id)
            .map_err(|_| "instance not found in the managed store".to_owned())?;
        match fs::remove_file(instance.root.join("schematics").join(&file_name)) {
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

/// 把一个投影文件从源实例复制到目标实例，用于跨版本共享。
/// 目标已有同名文件时报错，不覆盖实例自己的投影。
#[tauri::command]
pub(crate) async fn copy_schematic_to_instance(
    app: tauri::AppHandle,
    from_instance_id: String,
    to_instance_id: String,
    file_name: String,
) -> Result<String, String> {
    validate_file_name(&file_name)?;
    let from =
        InstanceId::parse(&from_instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let to = InstanceId::parse(&to_instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    if from == to {
        return Err("source and target instance are the same".to_owned());
    }
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let source = store
            .load(from)
            .map_err(|_| "source instance not found in the managed store".to_owned())?;
        let target = store
            .load(to)
            .map_err(|_| "target instance not found in the managed store".to_owned())?;

        let bytes = read_capped(&source.root.join("schematics").join(&file_name))?;
        let target_dir = target.root.join("schematics");
        fs::create_dir_all(&target_dir)
            .map_err(|_| "cannot prepare the target schematics directory".to_owned())?;
        let destination = target_dir.join(&file_name);
        if destination.exists() {
            return Err("the target instance already has a schematic with this name".to_owned());
        }
        fs::write(&destination, &bytes)
            .map_err(|_| "cannot copy the schematic into the target instance".to_owned())?;
        Ok(file_name)
    })
    .await
    .map_err(|_| "schematic copy task failed".to_owned())?
}

#[cfg(test)]
#[path = "schematics_tests.rs"]
mod tests;
