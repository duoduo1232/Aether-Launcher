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

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use aether_core::{GameInstance, InstanceId, InstanceState};
use aether_schematic::data_versions::nearest_release;
use aether_schematic::{LitematicMetadata, RegionData, parse, read_metadata};
use aether_storage::{InstanceStore, list_instances};
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

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
#[derive(Debug, Clone, Serialize)]
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

/// Litematica 在用户没填名字时写的占位值；按「没有名字」处理，界面回退到文件名。
const UNNAMED: &str = "unnamed";

impl SchematicMetadataDto {
    fn from_metadata(metadata: &LitematicMetadata, file_name: &str) -> Self {
        Self {
            name: display_name(&metadata.name, file_name),
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

/// 投影的展示名：元数据里没填（空白或 `Unnamed`）时用文件名去掉扩展名。
/// 很多投影的作者只填了作者和方块数，名字留着占位值，直接用会让一整列都叫 `Unnamed`。
fn display_name(name: &str, file_name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(UNNAMED) {
        let stem = file_name
            .strip_suffix(".litematic")
            .unwrap_or(file_name)
            .trim();
        if !stem.is_empty() {
            return stem.to_owned();
        }
    }
    name.to_owned()
}

/// 一个实例里的投影图列表项。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicSummary {
    /// 相对**所属来源目录**的路径，用 `/` 分隔（如 `城堡/主楼.litematic`）。
    relative_path: String,
    file_name: String,
    size_bytes: u64,
    /// 这份投影在哪个 `schematics/` 里：`instance` 是实例自己的，`version` 是
    /// 游戏目录下某个版本的，`game` 是游戏目录根上的。
    origin: &'static str,
    /// 来源所在的版本目录名；实例自己的和游戏目录根上的为空字符串。
    origin_version: String,
    metadata: SchematicMetadataDto,
}

/// 扫描时解析失败的文件，按稳定分类上报，不回显绝对路径。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedSchematic {
    relative_path: String,
    file_name: String,
    reason: &'static str,
    origin: &'static str,
    origin_version: String,
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
    /// 实际扫到的来源目录，按扫描顺序；界面据此说明「这些投影从哪读到的」。
    sources: Vec<SchematicSource>,
}

/// 扫描内容的说明，给界面显示「这些投影是从哪读到的」。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicSource {
    /// `instance` / `game` / `version`。
    origin: &'static str,
    /// 版本目录名；`instance` 和 `game` 为空。
    origin_version: String,
    /// 相对于游戏目录的描述，如 `schematics` 或 `versions/1.20.1/schematics`。
    label: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicListing {
    /// 只包含有投影（或解析失败文件）的实例；没有 `schematics/` 的版本不出现。
    instances: Vec<InstanceSchematics>,
    skipped: Vec<String>,
    /// 还没设置游戏目录的实例数（`game_directory` 字段加入之前创建的）。
    /// 这些实例扫不到任何东西，界面据此提示用户去设置。
    missing_game_directory: usize,
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

/// 一处待扫描的 `schematics/` 目录：**扫描来源**。
///
/// 投影在 `.minecraft` 里是被多个版本共用的，既可能放在游戏目录根上，
/// 也可能放在 `versions/<版本>/schematics`，还可能是实例自己独有的。
/// 三处都要看，否则用户明明看得到文件、界面却是空的。
#[derive(Debug, Clone, PartialEq, Eq)]
struct ScanSource {
    directory: PathBuf,
    origin: &'static str,
    /// 版本目录名；实例自己的和游戏目录根上的为空。
    origin_version: String,
}

/// 游戏目录下的 `schematics/` 根位置。
const ORIGIN_INSTANCE: &str = "instance";
const ORIGIN_GAME: &str = "game";
const ORIGIN_VERSION: &str = "version";

/// 解析一个实例真正使用的游戏目录。
///
/// 只认实例记录里的 `game_directory`：Aether 是独立启动器，不从别的启动器读
/// 配置，也不扫描磁盘去猜哪个目录是游戏目录。
///
/// 老记录（这个字段加入之前创建的实例）没有游戏目录，退回 `root/minecraft`。
/// 那种情况下投影会扫不到东西，界面会提示用户去设置游戏目录 —— 由用户明确
/// 选一次，然后写进记录。
pub(super) fn resolve_game_directory(instance: &GameInstance) -> PathBuf {
    instance.effective_game_directory()
}

/// 这个实例的游戏目录是不是还没设置（老记录）。
pub(super) fn missing_game_directory(instance: &GameInstance) -> bool {
    instance.game_directory.is_none()
}

/// 收集一个实例要扫描的全部来源，按优先级从高到低。
///
/// 游戏目录只来自实例记录；前端从不参与。
fn scan_sources(instance: &GameInstance) -> Vec<ScanSource> {
    let mut sources = vec![ScanSource {
        directory: instance.root.join("schematics"),
        origin: ORIGIN_INSTANCE,
        origin_version: String::new(),
    }];

    // 游戏目录就是实例记录的 `.minecraft`：原地读，不复制。
    let game_root = resolve_game_directory(instance);
    let shared = game_root.join("schematics");
    if !sources.iter().any(|source| source.directory == shared) {
        sources.push(ScanSource {
            directory: shared,
            origin: ORIGIN_GAME,
            origin_version: String::new(),
        });
    }

    // `versions/<版本>/schematics`：版本隔离的启动器把每个版本的投影放在这里。
    // 只读一层目录名，不递归进 `versions` 内部，避免把库目录当投影扫。
    let versions = game_root.join("versions");
    if let Ok(entries) = fs::read_dir(&versions) {
        let mut names: Vec<String> = entries
            .flatten()
            .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
            .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
            .collect();
        names.sort();
        for name in names {
            let directory = versions.join(&name).join("schematics");
            if !directory.is_dir() {
                continue;
            }
            if sources.iter().any(|source| source.directory == directory) {
                continue;
            }
            sources.push(ScanSource {
                directory,
                origin: ORIGIN_VERSION,
                origin_version: name,
            });
        }
    }

    sources
}

/// 扫描一个实例的所有来源目录，递归保留子文件夹结构。目录不存在视为没有投影；
/// 解析失败的文件按类别归入 `failed`，不中断扫描。
fn scan_schematics(instance: &GameInstance) -> (Vec<SchematicSummary>, Vec<FailedSchematic>) {
    let mut summaries = Vec::new();
    let mut failed = Vec::new();
    let mut budget = MAX_SCAN_ENTRIES;
    for source in scan_sources(instance) {
        walk_schematics(&source, "", 0, &mut budget, &mut summaries, &mut failed);
    }
    // 相对路径在同一个来源里唯一；跨来源按来源顺序排在后面。
    summaries.sort_by(|left, right| {
        (&left.origin, &left.origin_version, &left.relative_path).cmp(&(
            &right.origin,
            &right.origin_version,
            &right.relative_path,
        ))
    });
    failed.sort_by(|left, right| {
        (&left.origin, &left.origin_version, &left.relative_path).cmp(&(
            &right.origin,
            &right.origin_version,
            &right.relative_path,
        ))
    });
    (summaries, failed)
}

fn walk_schematics(
    source: &ScanSource,
    prefix: &str,
    depth: usize,
    budget: &mut usize,
    summaries: &mut Vec<SchematicSummary>,
    failed: &mut Vec<FailedSchematic>,
) {
    if depth > MAX_RELATIVE_DEPTH || *budget == 0 {
        return;
    }
    let entries = match fs::read_dir(&source.directory) {
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
            let nested = ScanSource {
                directory: path,
                origin: source.origin,
                origin_version: source.origin_version.clone(),
            };
            walk_schematics(&nested, &relative, depth + 1, budget, summaries, failed);
            continue;
        }
        if !metadata.is_file() || !name.ends_with(".litematic") {
            continue;
        }
        match summarize_file(&path, &relative, name, source) {
            Ok(summary) => summaries.push(summary),
            Err(reason) => failed.push(FailedSchematic {
                relative_path: relative,
                file_name: name.to_owned(),
                reason,
                origin: source.origin,
                origin_version: source.origin_version.clone(),
            }),
        }
    }
}

fn summarize_file(
    path: &Path,
    relative_path: &str,
    file_name: &str,
    source: &ScanSource,
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
        origin: source.origin,
        origin_version: source.origin_version.clone(),
        metadata: SchematicMetadataDto::from_metadata(&metadata, file_name),
    })
}
/// 只把真正扫出文件（或解析失败文件）的来源报给界面，空目录不占位置。
fn source_labels(
    instance: &GameInstance,
    summaries: &[SchematicSummary],
    failed: &[FailedSchematic],
) -> Vec<SchematicSource> {
    let mut labels = Vec::new();
    for source in scan_sources(instance) {
        let used = summaries.iter().any(|item| {
            item.origin == source.origin && item.origin_version == source.origin_version
        }) || failed.iter().any(|item| {
            item.origin == source.origin && item.origin_version == source.origin_version
        });
        if !used {
            continue;
        }
        labels.push(SchematicSource {
            label: source_label(&source),
            origin: source.origin,
            origin_version: source.origin_version,
        });
    }
    labels
}

/// 来源的相对描述：实例自己的报 `schematics`，其余报游戏目录下的相对路径。
fn source_label(source: &ScanSource) -> String {
    match source.origin {
        ORIGIN_VERSION => format!("versions/{}/schematics", source.origin_version),
        ORIGIN_GAME => "schematics".to_owned(),
        _ => "实例目录/schematics".to_owned(),
    }
}

/// 自动识别每个实例的所有 `schematics/`（实例自己的、游戏目录根上的、
/// `versions/<版本>/` 下的），按实例分组返回。
/// 没有投影也没有解析失败文件的实例直接跳过——列表里只出现有投影的版本。
///
/// # 为什么按游戏目录去重
///
/// 多个实例常常共用同一个 `.minecraft`（本机 134 个实例都指向 `E:\a\.minecraft`）。
/// 它们的 `versions/*/schematics` 完全一样，逐个实例扫一遍等于把同一个目录重复
/// 遍历上百次 —— 实测 134 个实例就能让页面一分钟都出不来。
///
/// 所以先用**游戏目录 + 实例私有目录**算出一个扫描键：同一个键只真正扫一次，
/// 结果按实例复用。实例自己的 `schematics/` 不共享，键里带上实例 ID 即可。
#[tauri::command]
pub(crate) async fn list_schematics(app: tauri::AppHandle) -> Result<SchematicListing, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing = list_instances(&root)
            .map_err(|error| format!("cannot scan the instance store: {error}"))?;

        // 已经扫过的游戏目录 -> 那一刻的扫描结果。同一目录只扫一次。
        let mut scanned: HashMap<ScanKey, (Vec<SchematicSummary>, Vec<FailedSchematic>)> =
            HashMap::new();
        let mut instances = Vec::new();

        for instance in &listing.instances {
            let key = ScanKey::of(instance);
            let (schematics, failed) = match scanned.get(&key) {
                Some(cached) => cached.clone(),
                None => {
                    let result = scan_schematics(instance);
                    scanned.insert(key, result.clone());
                    result
                }
            };
            if schematics.is_empty() && failed.is_empty() {
                continue;
            }
            // 只报真正扫出内容的来源，避免把空目录也列出来。
            let sources = source_labels(instance, &schematics, &failed);
            instances.push(InstanceSchematics {
                id: instance.id.to_string(),
                name: instance.name.clone(),
                version: instance.version.as_str().to_owned(),
                state: state_label(&instance.state),
                schematics,
                failed,
                sources,
            });
        }
        Ok(SchematicListing {
            instances,
            skipped: listing.skipped.iter().map(|s| s.id.clone()).collect(),
            missing_game_directory: listing
                .instances
                .iter()
                .filter(|instance| missing_game_directory(instance))
                .count(),
        })
    })
    .await
    .map_err(|_| "schematic listing task failed".to_owned())?
}

/// 决定「这次扫描能不能复用上次结果」的键。
///
/// 游戏目录相同、且实例私有目录相同，扫出来的东西就一样。
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct ScanKey {
    game_directory: PathBuf,
    instance_root: PathBuf,
}

impl ScanKey {
    fn of(instance: &GameInstance) -> Self {
        Self {
            game_directory: resolve_game_directory(instance),
            instance_root: instance.root.clone(),
        }
    }
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
        let metadata = SchematicMetadataDto::from_metadata(&schematic.metadata, &file_name);
        Ok(SchematicDetail {
            instance_id,
            relative_path,
            file_name,
            metadata,
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

/// 一次文件夹导入的结果。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicFolderImport {
    /// 成功复制进来的文件数。
    imported: usize,
    /// 跳过的文件，只带 `schematics/` 下的相对路径和稳定分类。
    skipped: Vec<SchematicFolderSkip>,
}

/// 一个被跳过的文件。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchematicFolderSkip {
    relative_path: String,
    reason: &'static str,
}

/// 把一个文件夹里的 `.litematic` 全部导入指定实例，保留子文件夹结构。
///
/// 文件夹由后端用系统选择框打开，前端只给实例 ID。重名文件自动改名，
/// 解析失败、超限或路径不安全的文件记进 `skipped`，不中断其余文件。
/// 取消选择时返回 `Ok(None)`。
#[tauri::command]
pub(crate) async fn import_schematic_folder(
    app: tauri::AppHandle,
    instance_id: String,
) -> Result<Option<SchematicFolderImport>, String> {
    let Some(source) = pick_folder(app.clone(), "选择包含 .litematic 的文件夹").await?
    else {
        return Ok(None);
    };
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

        let mut files = Vec::new();
        let mut budget = MAX_SCAN_ENTRIES;
        collect_litematics(&source, "", 0, &mut files, &mut budget);
        files.sort();

        let mut imported = 0usize;
        let mut skipped = Vec::new();
        for relative_path in files {
            match copy_one_schematic(&source, &directory, &relative_path) {
                Ok(()) => imported += 1,
                Err(reason) => skipped.push(SchematicFolderSkip {
                    relative_path,
                    reason,
                }),
            }
        }

        Ok(Some(SchematicFolderImport { imported, skipped }))
    })
    .await
    .map_err(|_| "schematic folder import task failed".to_owned())?
}

/// 后端打开系统文件夹选择框；取消时返回 `None`。
async fn pick_folder(
    app: tauri::AppHandle,
    title: &'static str,
) -> Result<Option<PathBuf>, String> {
    let picked = tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title(title).blocking_pick_folder()
    })
    .await
    .map_err(|error| format!("folder picker failed: {error}"))?;

    let Some(picked) = picked else {
        return Ok(None);
    };
    picked
        .into_path()
        .map(Some)
        .map_err(|_| "the selected folder path is not usable".to_owned())
}

/// 递归收集 `root` 下的 `.litematic`，返回 `/` 分隔的相对路径。
fn collect_litematics(
    root: &Path,
    prefix: &str,
    depth: usize,
    files: &mut Vec<String>,
    budget: &mut usize,
) {
    if depth > MAX_RELATIVE_DEPTH || *budget == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        if *budget == 0 {
            return;
        }
        *budget -= 1;
        let path = entry.path();
        // 符号链接可能指向文件夹之外，直接跳过。
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
            collect_litematics(&path, &relative, depth + 1, files, budget);
        } else if metadata.is_file() && name.ends_with(".litematic") {
            files.push(relative);
        }
    }
}

/// 复制一个投影文件到实例的 `schematics/`，保留子文件夹，重名自动改名。
fn copy_one_schematic(
    source_root: &Path,
    destination_root: &Path,
    relative_path: &str,
) -> Result<(), &'static str> {
    validate_relative_path(relative_path, true).map_err(|_| "unsafe-relative-path")?;

    let mut source = source_root.to_path_buf();
    for segment in relative_path.split('/') {
        source.push(segment);
    }
    let bytes = read_capped(&source).map_err(|_| "unreadable")?;
    // 先解析再落盘：坏文件不进列表，也不会在导入后显示成「解析失败」。
    read_metadata(&bytes).map_err(|_| "unparseable")?;

    let (folder, file_name) = match relative_path.rsplit_once('/') {
        Some((folder, name)) => (folder, name),
        None => ("", relative_path),
    };
    let mut directory = destination_root.to_path_buf();
    for segment in folder.split('/').filter(|segment| !segment.is_empty()) {
        directory.push(segment);
    }
    fs::create_dir_all(&directory).map_err(|_| "io")?;
    let destination = unique_destination(&directory, file_name).map_err(|_| "io")?;
    write_atomically(&directory, &destination, &bytes).map_err(|_| "io")
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

#[cfg(test)]
#[path = "schematics_sources_tests.rs"]
mod sources_tests;
