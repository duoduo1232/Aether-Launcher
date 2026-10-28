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
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use aether_core::{GameInstance, InstanceId, InstanceState};
use aether_schematic::data_versions::nearest_release;
use aether_schematic::{LitematicMetadata, RegionData, parse, read_metadata, read_metadata_from};
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

/// 解析投影头部时最多开几个线程。
///
/// 解压是纯 CPU 工作，开得比核数还多只会互相抢缓存；上限压住的是核数
/// 很多的机器上启动上百个线程的开销。
const MAX_PARSE_WORKERS: usize = 8;

/// 文件少于这个数就不值得开线程。
const PARALLEL_THRESHOLD: usize = 16;

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
///
/// **只用于写入**（导入、粘贴）：新文件一律落在实例自己的目录里。
fn schematic_path(instance: &GameInstance, relative: &str) -> PathBuf {
    join_relative(&instance.root.join("schematics"), relative)
}

/// 读/删用：按文件**实际所属的来源**定位，而不是一律当成实例私有目录。
///
/// 列表里的每个文件都带 `origin` / `originVersion`。忽略它们就会去找
/// `<实例>/schematics/...`，而文件其实在 `<游戏目录>/versions/<版本>/schematics/...`，
/// 于是一双击预览就报「cannot inspect schematic file」。
///
/// `origin` 只接受已知的三种取值，`originVersion` 必须是单个安全路径段，
/// 拼出来的路径还必须仍在该来源目录之内 —— 前端永远不能借此读到别处。
fn resolve_existing_path(
    instance: &GameInstance,
    origin: &str,
    origin_version: &str,
    relative: &str,
) -> Result<PathBuf, String> {
    let base = match origin {
        ORIGIN_INSTANCE => instance.root.join("schematics"),
        ORIGIN_GAME => resolve_game_directory(instance).join("schematics"),
        ORIGIN_VERSION => {
            if !is_single_segment(origin_version) {
                return Err("invalid version name".to_owned());
            }
            resolve_game_directory(instance)
                .join("versions")
                .join(origin_version)
                .join("schematics")
        }
        _ => return Err("unknown schematic source".to_owned()),
    };
    Ok(join_relative(&base, relative))
}

/// 逐段拼接，拒绝空段、`.`、`..`（`validate_relative_path` 已经查过，
/// 这里再守一次，避免调用方漏掉校验）。
fn join_relative(base: &Path, relative: &str) -> PathBuf {
    let mut path = base.to_path_buf();
    for segment in relative.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            continue;
        }
        path.push(segment);
    }
    path
}

/// 版本目录名必须是单个安全路径段。
fn is_single_segment(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.contains(['/', '\\', ':'])
        && !name.contains("..")
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

/// 一个实例在当前磁盘状态下要扫描的全部来源。
///
/// 生产路径不走这里（[`list_schematics`] 会缓存 `versions/` 的枚举结果），
/// 保留它是因为测试要按「给定实例」推导来源，比手写路径可靠。
#[cfg(test)]
fn scan_sources(instance: &GameInstance) -> Vec<ScanSource> {
    let game_root = resolve_game_directory(instance);
    let versions = version_directories(&game_root);
    game_sources(
        &instance.root,
        &game_root,
        instance.version.as_str(),
        &versions,
    )
}

/// 打开一个独立的 3D 预览窗口，返回新窗口的标签。
///
/// 预览放在单独窗口里，是因为它的操作（拖动旋转、滚轮缩放、右键平移）和主
/// 界面的滚动/多选相冲突；独立窗口也能同时查看多个投影。
///
/// 窗口里加载的是同一个前端，靠 URL 片段 `#/schematic-preview?instance=…`
/// 走另一条路由 —— 前端从不传 HTML 或脚本，只传实例 ID、相对路径与来源。
#[tauri::command]
pub(crate) async fn open_schematic_window(
    app: tauri::AppHandle,
    instance_id: String,
    relative_path: String,
    origin: String,
    origin_version: String,
    title: String,
) -> Result<String, String> {
    validate_relative_path(&relative_path, true)?;
    // 实例 ID 先校验形状，避免把任意字符串拼进窗口 URL。
    let id = InstanceId::parse(&instance_id).map_err(|_| "invalid instance ID".to_owned())?;
    // 标题会显示在窗口栏上，只保留一行、限长，防止塞入控制字符或超长文本。
    let title: String = title.chars().filter(|c| !c.is_control()).take(80).collect();

    let label = format!("preview-{}", next_preview_label());
    let query = format!(
        "#/schematic-preview?instance={}&path={}&origin={}&version={}&title={}",
        id,
        url_encode(&relative_path),
        url_encode(&origin),
        url_encode(&origin_version),
        url_encode(&title),
    );

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(query.into()))
        .title(title)
        .inner_size(1100.0, 780.0)
        .min_inner_size(720.0, 520.0)
        .build()
        .map_err(|error| format!("cannot open the preview window: {error}"))?;

    Ok(label)
}

/// 预览窗口的自增序号，保证标签唯一（Tauri 拒绝重复标签）。
fn next_preview_label() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// 最小百分号编码：只放行 URL 里安全的字符。
fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
/// `versions/` 下的全部版本目录名，排序后返回。
/// 多个实例共用同一个 `.minecraft` 时这一层目录完全一样，所以单独提出来，
/// 由调用方按游戏目录缓存一次 —— 否则 134 个实例会各枚举一遍、每次都
/// `is_dir` 上百次（实测 816 ms）。
fn version_directories(game_root: &Path) -> Vec<String> {
    let versions = game_root.join("versions");
    let Ok(entries) = fs::read_dir(&versions) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| entry.file_name().to_str().map(str::to_owned))
        .collect();
    names.sort();
    names
}

/// 用**已枚举好**的版本目录名拼出一个实例的扫描来源。
///
/// 一个实例的投影来源有三处，缺一处都会出现「文件管理器里有、界面是空的」：
///
/// 1. 实例自己的 `<实例目录>/schematics`；
/// 2. 游戏目录根上的 `<游戏目录>/schematics`——不启用版本隔离的启动器
///    （PCL2 默认、HMCL 默认）把投影放这里，所有版本共用；
/// 3. **这个实例自己那个版本**的 `<游戏目录>/versions/<版本>/schematics`。
///
/// 第 3 条**只认实例记录的 `version`**，不能把 `versions/` 下所有版本的
/// `schematics` 都算进来：那样每个版本都会显示整个 `.minecraft` 里的全部投影
/// （本机 134 个实例全部显示 1649 个，用户看到的就是「每个版本都有 1000 多个」）。
/// 投影跟着版本走，`versions/生电/schematics` 只属于生电那个版本。
///
/// `versions` 是已枚举好的版本目录名，用于判断这个实例的版本目录是否存在，
/// 不需要再逐个 `is_dir`。
fn game_sources(
    instance_root: &Path,
    game_root: &Path,
    version: &str,
    versions: &[String],
) -> Vec<ScanSource> {
    let mut sources = vec![ScanSource {
        directory: instance_root.join("schematics"),
        origin: ORIGIN_INSTANCE,
        origin_version: String::new(),
    }];

    // 游戏目录就是实例记录的 `.minecraft`：原地读，不复制。
    let shared = game_root.join("schematics");
    if !sources.iter().any(|source| source.directory == shared) {
        sources.push(ScanSource {
            directory: shared,
            origin: ORIGIN_GAME,
            origin_version: String::new(),
        });
    }

    // 只加这个实例自己的版本目录；版本名必须真的在 `versions/` 下存在。
    if versions.iter().any(|name| name == version) {
        let directory = game_root.join("versions").join(version).join("schematics");
        if directory.is_dir() && !sources.iter().any(|source| source.directory == directory) {
            sources.push(ScanSource {
                directory,
                origin: ORIGIN_VERSION,
                origin_version: version.to_owned(),
            });
        }
    }

    sources
}

/// 扫描**一个**来源目录，递归保留子文件夹结构。目录不存在视为没有投影；
/// 解析失败的文件按类别归入 `failed`，不中断扫描。
///
/// 结果与实例无关，所以调用方可以按目录路径缓存，让上百个共用同一个
/// `.minecraft` 的实例只遍历一次。
///
/// # 为什么分两趟
///
/// 遍历目录是 IO 串行的工作，解压头部是纯 CPU 的工作。先把待解析的文件收集
/// 起来，再丢给线程池并行解析：本机 1723 个投影单线程 1.56 s，8 线程 0.50 s。
/// 图省事在一个循环里做，就是用户看到的「卡加载」。
fn scan_directory(source: &ScanSource) -> DirectoryScan {
    let mut pending: Vec<PendingFile> = Vec::new();
    let mut failed = Vec::new();
    let mut budget = MAX_SCAN_ENTRIES;
    walk_schematics(source, "", 0, &mut budget, &mut pending, &mut failed);

    let schematics = parse_pending(pending, &mut failed);
    let mut scan = DirectoryScan { schematics, failed };
    scan.sort();
    scan
}

impl DirectoryScan {
    /// 同一目录内的顺序：按相对路径，保证跨实例复用后顺序一致。
    fn sort(&mut self) {
        self.schematics
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
        self.failed
            .sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    }
}

/// 一个等着解析的投影文件，已经带上它所属的来源。
struct PendingFile {
    path: PathBuf,
    relative_path: String,
    file_name: String,
    origin: &'static str,
    origin_version: String,
    size_bytes: u64,
}

/// 并行解析收集到的文件，返回逐个成功/失败的结果。
///
/// 每个文件只读头部（[`read_metadata_from`]），失败不影响其它文件；失败的
/// 那些必须原样报给界面，不能悄悄吞掉 —— 用户看得到文件却看不到它，比报
/// 「无法解析」更糟。
fn parse_pending(
    pending: Vec<PendingFile>,
    failed: &mut Vec<FailedSchematic>,
) -> Vec<SchematicSummary> {
    if pending.is_empty() {
        return Vec::new();
    }
    // 投影多到值得并行时再开线程：几个文件时反而不如串行。
    let workers = std::thread::available_parallelism()
        .map(|count| count.get())
        .unwrap_or(1)
        .min(MAX_PARSE_WORKERS);
    if workers <= 1 || pending.len() < PARALLEL_THRESHOLD {
        let outcomes: Vec<_> = pending.into_iter().map(parse_one_pending).collect();
        return collect_outcomes(outcomes, failed);
    }

    let chunk = pending.len().div_ceil(workers);
    let mut batches: Vec<Vec<PendingFile>> = Vec::new();
    let mut rest = pending;
    while !rest.is_empty() {
        let tail = rest.split_off(chunk.min(rest.len()));
        batches.push(std::mem::replace(&mut rest, tail));
    }

    let handles: Vec<_> = batches
        .into_iter()
        .map(|batch| {
            std::thread::spawn(move || batch.into_iter().map(parse_one_pending).collect::<Vec<_>>())
        })
        .collect();

    // join 顺序与线程调度无关，所以结果稳定；排序在调用方统一做。
    let mut outcomes = Vec::new();
    for handle in handles {
        outcomes.extend(handle.join().unwrap_or_default());
    }
    collect_outcomes(outcomes, failed)
}

/// 把逐个文件的解析结果拆成「投影列表」和「失败列表」。
fn collect_outcomes(
    outcomes: Vec<ParseOutcome>,
    failed: &mut Vec<FailedSchematic>,
) -> Vec<SchematicSummary> {
    let mut summaries = Vec::new();
    for outcome in outcomes {
        match outcome {
            ParseOutcome::Ok(summary) => summaries.push(summary),
            ParseOutcome::Failed(entry) => failed.push(entry),
        }
    }
    summaries
}

/// 一个文件的解析结果：要么是投影摘要，要么是一条失败记录。
enum ParseOutcome {
    Ok(SchematicSummary),
    Failed(FailedSchematic),
}

/// 解析一个文件，成功返回摘要，失败返回归类后的失败记录。
///
/// 读取失败报 `unreadable`、NBT 解不开报 `unparseable`，与
/// [`queue_file`] 的 `oversized` 合起来就是界面能看到的全部失败原因。
fn parse_one_pending(file: PendingFile) -> ParseOutcome {
    let failed = |reason: &'static str| {
        ParseOutcome::Failed(FailedSchematic {
            relative_path: file.relative_path.clone(),
            file_name: file.file_name.clone(),
            reason,
            origin: file.origin,
            origin_version: file.origin_version.clone(),
        })
    };
    let Ok(handle) = fs::File::open(&file.path) else {
        return failed("unreadable");
    };
    let Ok(metadata) = read_metadata_from(std::io::BufReader::new(handle)) else {
        return failed("unparseable");
    };
    ParseOutcome::Ok(SchematicSummary {
        relative_path: file.relative_path,
        file_name: file.file_name.clone(),
        size_bytes: file.size_bytes,
        origin: file.origin,
        origin_version: file.origin_version,
        metadata: SchematicMetadataDto::from_metadata(&metadata, &file.file_name),
    })
}

/// 递归遍历一个来源目录，把可以解析的投影文件收进 `pending`。
///
/// 这里只做目录遍历和元数据判断，不解析文件内容 —— 解析在
/// [`parse_pending`] 里并行完成。
fn walk_schematics(
    source: &ScanSource,
    prefix: &str,
    depth: usize,
    budget: &mut usize,
    pending: &mut Vec<PendingFile>,
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
            walk_schematics(&nested, &relative, depth + 1, budget, pending, failed);
            continue;
        }
        if !metadata.is_file() || !name.ends_with(".litematic") {
            continue;
        }
        match queue_file(&path, &relative, name, source, metadata.len()) {
            Ok(file) => pending.push(file),
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

/// 只做扫描阶段就能判定的检查（大小上限），然后把文件排进待解析队列。
///
/// 真正的 NBT 解析在 [`parse_pending`] 里做：那里是并行执行的，而且解析
/// 失败的分类和这里不同（`parse_one_pending` 返回 `None`）。
fn queue_file(
    path: &Path,
    relative_path: &str,
    file_name: &str,
    source: &ScanSource,
    size_bytes: u64,
) -> Result<PendingFile, &'static str> {
    if size_bytes > MAX_SCHEMATIC_FILE_BYTES {
        return Err("oversized");
    }
    Ok(PendingFile {
        path: path.to_path_buf(),
        relative_path: relative_path.to_owned(),
        file_name: file_name.to_owned(),
        origin: source.origin,
        origin_version: source.origin_version.clone(),
        size_bytes,
    })
}

/// 只把真正扫出文件（或解析失败文件）的来源报给界面，空目录不占位置。
///
/// `sources` 由调用方算好传进来：重复调用 [`scan_sources`] 会重新枚举
/// `versions/` 下每一个目录，134 个实例就是几百毫秒的纯浪费。
fn source_labels(
    sources: &[ScanSource],
    summaries: &[SchematicSummary],
    failed: &[FailedSchematic],
) -> Vec<SchematicSource> {
    let mut labels = Vec::new();
    for source in sources {
        let used = summaries.iter().any(|item| {
            item.origin == source.origin && item.origin_version == source.origin_version
        }) || failed.iter().any(|item| {
            item.origin == source.origin && item.origin_version == source.origin_version
        });
        if !used {
            continue;
        }
        labels.push(SchematicSource {
            label: source_label(source),
            origin: source.origin,
            origin_version: source.origin_version.clone(),
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
/// 所以先用**游戏目录**算出一个扫描键：同一个键只真正扫一次，结果按实例复用。
///
/// # 为什么版本列表也要缓存
///
/// 光去重扫描还不够：每个实例都得先算出「要扫哪些来源」，而算来源要枚举
/// `versions/` 下的每个目录并逐个 `is_dir`。本机 134 个实例 × 26 个候选目录
/// 实测 816 ms。`versions/` 的内容只取决于游戏目录，所以按游戏目录缓存一份
/// 目录名，之后每个实例只是拼几个路径。
#[tauri::command]
pub(crate) async fn list_schematics(app: tauri::AppHandle) -> Result<SchematicListing, String> {
    let root = instance_store_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        let listing = list_instances(&root)
            .map_err(|error| format!("cannot scan the instance store: {error}"))?;

        // 游戏目录 -> 该目录下 `versions/` 的目录名（只读一次盘）。
        let mut version_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
        // 来源目录 -> 这一层扫出来的文件；同一个目录只遍历一次。
        //
        // **不能按实例或按游戏目录缓存整份结果**：共用同一个 `.minecraft` 的
        // 实例来源几乎一样，但实例私有目录各不相同；按游戏目录缓存整份结果
        // 会让 134 个实例全部显示同一份上千个投影。
        let mut by_directory: HashMap<PathBuf, Arc<DirectoryScan>> = HashMap::new();
        let mut instances = Vec::new();

        for instance in &listing.instances {
            let game_root = resolve_game_directory(instance);
            let versions = version_cache
                .entry(game_root.clone())
                .or_insert_with(|| version_directories(&game_root));
            let sources = game_sources(
                &instance.root,
                &game_root,
                instance.version.as_str(),
                versions,
            );

            let mut result = ScanResult::default();
            for source in &sources {
                let scan = match by_directory.get(&source.directory) {
                    Some(cached) => Arc::clone(cached),
                    None => {
                        let scan = Arc::new(scan_directory(source));
                        by_directory.insert(source.directory.clone(), Arc::clone(&scan));
                        scan
                    }
                };
                result.absorb(&scan, source);
            }
            result.sort();

            if result.schematics.is_empty() && result.failed.is_empty() {
                continue;
            }
            // 只报真正扫出内容的来源，避免把空目录也列出来。
            let labels = source_labels(&sources, &result.schematics, &result.failed);
            instances.push(InstanceSchematics {
                id: instance.id.to_string(),
                name: instance.name.clone(),
                version: instance.version.as_str().to_owned(),
                state: state_label(&instance.state),
                schematics: result.schematics,
                failed: result.failed,
                sources: labels,
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

/// 一个实例的扫描结果：它所有来源目录里的文件合起来。
#[derive(Debug, Default)]
struct ScanResult {
    schematics: Vec<SchematicSummary>,
    failed: Vec<FailedSchematic>,
}

impl ScanResult {
    /// 把一个**已扫描的目录**并入本实例的结果。
    ///
    /// 目录层的结果可以跨实例复用（键是目录路径），但 `origin` /
    /// `originVersion` 是「这个目录属于哪个实例」的信息，不同实例可能不同
    /// （比如两个实例的私有目录同一路径、或游戏目录配得不一样），所以
    /// 拼接时逐条套用当前实例的来源标签，而不是直接搬用缓存里的。
    fn absorb(&mut self, scan: &DirectoryScan, source: &ScanSource) {
        self.schematics.extend(scan.schematics.iter().map(|item| {
            let mut item = item.clone();
            item.origin = source.origin;
            item.origin_version = source.origin_version.clone();
            item
        }));
        self.failed
            .extend(scan.failed.iter().map(|item| FailedSchematic {
                relative_path: item.relative_path.clone(),
                file_name: item.file_name.clone(),
                reason: item.reason,
                origin: source.origin,
                origin_version: source.origin_version.clone(),
            }));
    }

    /// 相对路径在同一个来源里唯一；跨来源按来源顺序排在后面。
    fn sort(&mut self) {
        self.schematics.sort_by(|left, right| {
            (&left.origin, &left.origin_version, &left.relative_path).cmp(&(
                &right.origin,
                &right.origin_version,
                &right.relative_path,
            ))
        });
        self.failed.sort_by(|left, right| {
            (&left.origin, &left.origin_version, &left.relative_path).cmp(&(
                &right.origin,
                &right.origin_version,
                &right.relative_path,
            ))
        });
    }
}

/// 单个来源目录扫出来的原始文件，与实例无关，可以跨实例复用。
#[derive(Debug, Default)]
struct DirectoryScan {
    schematics: Vec<SchematicSummary>,
    failed: Vec<FailedSchematic>,
}

/// 读取并完整解码指定实例里的一个投影文件，返回预览数据。
///
/// `origin` / `originVersion` 必须与列表里那一项一致：文件可能位于实例私有
/// 目录、游戏目录根或某个版本目录下，后端据此定位（见 [`resolve_existing_path`]）。
#[tauri::command]
pub(crate) async fn read_schematic(
    app: tauri::AppHandle,
    instance_id: String,
    relative_path: String,
    origin: String,
    origin_version: String,
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
        let path = resolve_existing_path(&instance, &origin, &origin_version, &relative_path)?;
        let bytes = read_capped(&path)?;
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
///
/// 与读取一样按 `origin` 定位：用户在界面上看到的是哪个来源的文件，删的就是
/// 那个来源的那个文件。
#[tauri::command]
pub(crate) async fn delete_schematic(
    app: tauri::AppHandle,
    instance_id: String,
    relative_path: String,
    origin: String,
    origin_version: String,
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
        let path = resolve_existing_path(&instance, &origin, &origin_version, &relative_path)?;
        match fs::remove_file(&path) {
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
///
/// 参数多是因为 Tauri 命令的参数就是 IPC 契约：源（实例 + 相对路径 + 来源）
/// 和目标（实例 + 文件夹 + 模式）分开传，比塞一个嵌套结构更直接。
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub(crate) async fn paste_schematic(
    app: tauri::AppHandle,
    from_instance_id: String,
    from_path: String,
    from_origin: String,
    from_origin_version: String,
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

        // 源可能来自游戏目录或某个版本目录；目标**永远**是实例自己的目录，
        // 这样粘贴只会往实例里加文件，不会改动共享的游戏目录。
        let source = resolve_existing_path(
            &source_instance,
            &from_origin,
            &from_origin_version,
            &from_path,
        )?;
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
