//! 投影图（`.litematic`，Litematica「投影」）命令：自动识别实例里的投影。
//!
//! 投影文件跟实例走：每个实例游戏目录下的 `schematics/` 都会被自动扫描，
//! **子文件夹结构原样保留**——每个文件用 `relativePath`（`/` 分隔）标识，
//! 前端据此还原目录树。只有真正有投影（或解析失败文件）的实例才会出现在
//! 列表里，没有 `schematics/` 目录的版本不占位置。
//!
//! IPC 只传实例 ID 和 `schematics/` 下的相对路径；实例目录来自受信任的实例
//! 存储，前端从不给绝对路径。导入走 `import_schematic`：前端读取用户选择的
//! 文件**内容**传进来，后端按安全文件名写进指定实例；复制/移动走
//! `paste_schematic`，同实例内整理目录和跨版本共享是同一套逻辑。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use aether_core::{GameInstance, InstanceId, InstanceState};
use aether_schematic::data_versions::nearest_release;
use aether_schematic::{LitematicMetadata, RegionData, parse, read_metadata};
use aether_storage::{InstanceStore, list_instances};
use serde::Serialize;
use tauri::Manager;

/// 单个投影文件的读取上限。
const MAX_SCHEMATIC_FILE_BYTES: u64 = 64 * 1024 * 1024;

/// `schematics/` 下相对路径的长度与层级上限。
const MAX_RELATIVE_PATH_LEN: usize = 1024;
const MAX_RELATIVE_DEPTH: usize = 8;

/// 一次扫描最多访问的目录条目数，防止超大目录拖住 UI。
const MAX_SCAN_ENTRIES: usize = 4096;

/// 同名文件自动改名时的最大尝试次数。
const MAX_RENAME_ATTEMPTS: u32 = 999;

/// 临时文件名里的自增序号，避免并发写入撞到同一个 staging 文件。
static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

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
    /// `schematics/` 下的相对路径，用 `/` 分隔（如 `城堡/主楼.litematic`）。
    relative_path: String,
    file_name: String,
    size_bytes: u64,
    metadata: SchematicMetadataDto,
}

/// 扫描时解析失败的文件，按稳定分类上报，不回显绝对路径。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedSchematic {
    relative_path: String,
    file_name: String,
    reason: &'static str,
}

/// 一个有投影的实例（版本）自动识别出的全部投影图。
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
    /// 只包含有投影（或解析失败文件）的实例；没有 `schematics/` 的版本不出现。
    instances: Vec<InstanceSchematics>,
    skipped: Vec<String>,
}

/// 完整解码结果（预览数据）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicDetail {
    instance_id: String,
    relative_path: String,
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

/// IPC 只接受 `schematics/` 下的相对路径：`/` 分隔、不含 `..`、盘符、
/// 保留名或结尾点，层级和长度都有上限。
fn validate_relative_path(path: &str, require_litematic: bool) -> Result<(), String> {
    if path.is_empty() {
        return Err("schematic path must not be empty".to_owned());
    }
    if path.len() > MAX_RELATIVE_PATH_LEN {
        return Err("schematic path is too long".to_owned());
    }
    if path.contains('\\') {
        return Err("schematic path must use forward slashes".to_owned());
    }
    if require_litematic && (!path.ends_with(".litematic") || path == ".litematic") {
        return Err("schematic path must end with .litematic".to_owned());
    }
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() > MAX_RELATIVE_DEPTH + 1 {
        return Err("schematic path is nested too deeply".to_owned());
    }
    if segments.iter().any(|segment| segment.len() > 255) {
        return Err("schematic path segment is too long".to_owned());
    }
    if !aether_download::is_safe_relative_path(Path::new(path)) {
        return Err("schematic path is not a safe relative path".to_owned());
    }
    Ok(())
}

/// 目标文件夹：空字符串表示 `schematics/` 根目录。
fn validate_folder_path(folder: &str) -> Result<(), String> {
    if folder.is_empty() {
        return Ok(());
    }
    validate_relative_path(folder, false)
}

/// 把相对路径拼到实例的 `schematics/` 下。逐段 push，不依赖平台分隔符。
fn schematic_path(instance: &GameInstance, relative: &str) -> PathBuf {
    let mut path = instance.root.join("schematics");
    for segment in relative.split('/') {
        path.push(segment);
    }
    path
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

/// 先写临时文件再原子改名，避免半截文件被当作可解析的投影。
fn write_atomically(directory: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("schematic");
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let staging = directory.join(format!(
        ".staging-{}-{sequence}-{name}.tmp",
        std::process::id()
    ));
    let result = (|| {
        fs::write(&staging, bytes).map_err(|_| "cannot write the staged schematic".to_owned())?;
        fs::rename(&staging, destination).map_err(|_| "cannot publish the schematic".to_owned())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&staging);
    }
    result
}

/// 目标目录里挑一个不冲突的文件名：`name.litematic` → `name (2).litematic` …
fn unique_destination(directory: &Path, file_name: &str) -> Result<PathBuf, String> {
    let candidate = directory.join(file_name);
    if !candidate.exists() {
        return Ok(candidate);
    }
    let stem = file_name.strip_suffix(".litematic").unwrap_or(file_name);
    for index in 2..=MAX_RENAME_ATTEMPTS {
        let candidate = directory.join(format!("{stem} ({index}).litematic"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err("too many files with the same name in the target folder".to_owned())
}

/// 扫描一个实例的 `schematics/` 目录，递归保留子文件夹结构。目录不存在视为
/// 没有投影；解析失败的文件按类别归入 `failed`，不中断扫描。
fn scan_schematics(instance: &GameInstance) -> (Vec<SchematicSummary>, Vec<FailedSchematic>) {
    let mut summaries = Vec::new();
    let mut failed = Vec::new();
    let mut budget = MAX_SCAN_ENTRIES;
    walk_schematics(
        &instance.root.join("schematics"),
        "",
        0,
        &mut budget,
        &mut summaries,
        &mut failed,
    );
    summaries.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    failed.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    (summaries, failed)
}

fn walk_schematics(
    directory: &Path,
    prefix: &str,
    depth: usize,
    budget: &mut usize,
    summaries: &mut Vec<SchematicSummary>,
    failed: &mut Vec<FailedSchematic>,
) {
    if depth > MAX_RELATIVE_DEPTH || *budget == 0 {
        return;
    }
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let path = entry.path();
        // 符号链接可能指向实例目录之外，直接跳过，不让扫描越界。
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        if metadata.file_type().is_symlink() {
            continue;
        }
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let relative = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        if relative.len() > MAX_RELATIVE_PATH_LEN {
            continue;
        }
        if metadata.is_dir() {
            walk_schematics(&path, &relative, depth + 1, budget, summaries, failed);
            continue;
        }
        if !metadata.is_file() || !name.ends_with(".litematic") {
            continue;
        }
        match summarize_file(&path, &relative, name) {
            Ok(summary) => summaries.push(summary),
            Err(reason) => failed.push(FailedSchematic {
                relative_path: relative,
                file_name: name.to_owned(),
                reason,
            }),
        }
    }
}

fn summarize_file(
    path: &Path,
    relative_path: &str,
    file_name: &str,
) -> Result<SchematicSummary, &'static str> {
    let size = fs::metadata(path).map_err(|_| "unreadable")?.len();
    if size > MAX_SCHEMATIC_FILE_BYTES {
        return Err("oversized");
    }
    let bytes = fs::read(path).map_err(|_| "unreadable")?;
    let metadata = read_metadata(&bytes).map_err(|_| "unparseable")?;
    Ok(SchematicSummary {
        relative_path: relative_path.to_owned(),
        file_name: file_name.to_owned(),
        size_bytes: size,
        metadata: SchematicMetadataDto::from_metadata(&metadata),
    })
}

/// 自动识别每个实例游戏目录 `schematics/` 下的投影文件，按实例分组返回。
/// 没有投影也没有解析失败文件的实例直接跳过——列表里只出现有投影的版本。
#[tauri::command]
pub(crate) async fn list_schematics(app: tauri::AppHandle) -> Result<SchematicListing, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing = list_instances(&root)
            .map_err(|error| format!("cannot scan the instance store: {error}"))?;
        let mut instances = Vec::new();
        for instance in &listing.instances {
            let (schematics, failed) = scan_schematics(instance);
            if schematics.is_empty() && failed.is_empty() {
                continue;
            }
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
    relative_path: String,
) -> Result<SchematicDetail, String> {
    validate_relative_path(&relative_path, true)?;
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let instance = store
            .load(id)
            .map_err(|_| "instance not found in the managed store".to_owned())?;
        let bytes = read_capped(&schematic_path(&instance, &relative_path))?;
        let schematic = parse(&bytes).map_err(|error| error.to_string())?;
        let file_name = relative_path
            .rsplit('/')
            .next()
            .unwrap_or(&relative_path)
            .to_owned();
        Ok(SchematicDetail {
            instance_id,
            relative_path,
            file_name,
            metadata: SchematicMetadataDto::from_metadata(&schematic.metadata),
            regions: schematic.regions,
        })
    })
    .await
    .map_err(|_| "schematic read task failed".to_owned())?
}

/// 把文件内容写进指定实例的 `schematics/` 根目录；同名文件已存在时拒绝覆盖。
#[tauri::command]
pub(crate) async fn import_schematic(
    app: tauri::AppHandle,
    instance_id: String,
    file_name: String,
    bytes: Vec<u8>,
) -> Result<String, String> {
    validate_relative_path(&file_name, true)?;
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
        write_atomically(&directory, &destination, &bytes)?;
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
    relative_path: String,
) -> Result<(), String> {
    validate_relative_path(&relative_path, true)?;
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let instance = store
            .load(id)
            .map_err(|_| "instance not found in the managed store".to_owned())?;
        match fs::remove_file(schematic_path(&instance, &relative_path)) {
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

/// 把投影复制或移动到目标文件夹，返回新的相对路径。
///
/// 源和目标可以是同一个实例（整理目录），也可以是不同实例（跨版本共享）。
/// 目标已有同名文件时自动改名成 `名字 (2).litematic`，不覆盖已有投影。
#[tauri::command]
pub(crate) async fn paste_schematic(
    app: tauri::AppHandle,
    from_instance_id: String,
    from_path: String,
    to_instance_id: String,
    to_folder: String,
    move_file: bool,
) -> Result<String, String> {
    validate_relative_path(&from_path, true)?;
    validate_folder_path(&to_folder)?;
    let from =
        InstanceId::parse(&from_instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let to = InstanceId::parse(&to_instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let store =
            InstanceStore::open(&root).map_err(|_| "cannot open the instance store".to_owned())?;
        let source_instance = store
            .load(from)
            .map_err(|_| "source instance not found in the managed store".to_owned())?;
        let target_instance = store
            .load(to)
            .map_err(|_| "target instance not found in the managed store".to_owned())?;

        let source = schematic_path(&source_instance, &from_path);
        let bytes = read_capped(&source)?;

        let target_directory = if to_folder.is_empty() {
            target_instance.root.join("schematics")
        } else {
            schematic_path(&target_instance, &to_folder)
        };
        fs::create_dir_all(&target_directory)
            .map_err(|_| "cannot prepare the target schematics folder".to_owned())?;

        let file_name = from_path.rsplit('/').next().unwrap_or(&from_path);
        let destination = unique_destination(&target_directory, file_name)?;
        write_atomically(&target_directory, &destination, &bytes)?;

        if move_file {
            // 目标名一定和源不同（`unique_destination` 保证），删源不会误删刚写的文件。
            fs::remove_file(&source)
                .map_err(|_| "cannot remove the source schematic".to_owned())?;
        }

        let new_name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(file_name);
        Ok(if to_folder.is_empty() {
            new_name.to_owned()
        } else {
            format!("{to_folder}/{new_name}")
        })
    })
    .await
    .map_err(|_| "schematic paste task failed".to_owned())?
}

#[cfg(test)]
#[path = "schematics_tests.rs"]
mod tests;
