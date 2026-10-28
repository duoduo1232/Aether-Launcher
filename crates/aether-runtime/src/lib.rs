use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JavaRuntime {
    pub executable: PathBuf,
    pub major_version: u16,
    pub architecture: String,
    pub vendor: Option<String>,
}

impl JavaRuntime {
    #[must_use]
    pub fn supports(&self, required_major_version: u16) -> bool {
        self.major_version == required_major_version
    }

    /// Probe a Java executable and use its JVM properties when available.
    pub fn probe(executable: PathBuf) -> Result<Self, RuntimeError> {
        let mut output = Command::new(&executable)
            .args(["-XshowSettings:properties", "-version"])
            .output()
            .map_err(|error| RuntimeError::CommandFailed(executable.clone(), error))?;

        // Some older VMs do not support -XshowSettings:properties.
        if !output.status.success() {
            output = Command::new(&executable)
                .arg("-version")
                .output()
                .map_err(|error| RuntimeError::CommandFailed(executable.clone(), error))?;
        }

        if !output.status.success() {
            return Err(RuntimeError::UnsuccessfulExit(executable, output.status));
        }

        // The version banner normally goes to stderr, but some VMs use stdout.
        let version_output = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );

        let major_version = parse_java_major_version(&version_output)
            .ok_or_else(|| RuntimeError::VersionParseFailure(executable.clone()))?;

        Ok(Self {
            executable,
            major_version,
            architecture: extract_architecture(&version_output),
            vendor: extract_vendor(&version_output),
        })
    }
}

#[derive(Debug, Error)]
pub enum RuntimeError {
    #[error("failed to execute java command at {0:?}: {1}")]
    CommandFailed(PathBuf, std::io::Error),
    #[error("failed to parse java version from {0:?}")]
    VersionParseFailure(PathBuf),
    #[error("java command at {0:?} exited unsuccessfully: {1}")]
    UnsuccessfulExit(PathBuf, std::process::ExitStatus),
}

fn parse_java_major_version(output: &str) -> Option<u16> {
    for line in output.lines() {
        let line = line.trim();
        let Some((prefix, quoted)) = line.split_once('"') else {
            continue;
        };
        if !matches!(
            prefix.trim(),
            "java version" | "openjdk version" | "java full version"
        ) {
            continue;
        }

        let version = quoted.split('"').next()?;
        let mut parts = version.split('.');
        let first = parts.next()?;
        if first == "1" {
            // Java 8 and earlier used versions such as 1.8.0_391.
            return parts.next()?.parse::<u16>().ok();
        }

        // Early-access JVMs may advertise versions such as "25-ea".
        let digits: String = first.chars().take_while(|c| c.is_ascii_digit()).collect();
        if let Ok(major) = digits.parse::<u16>() {
            return Some(major);
        }
    }
    None
}

fn extract_property<'a>(output: &'a str, property: &str) -> Option<&'a str> {
    output.lines().find_map(|line| {
        let (name, value) = line.trim().split_once('=')?;
        (name.trim() == property)
            .then_some(value.trim())
            .filter(|value| !value.is_empty())
    })
}

fn extract_architecture(output: &str) -> String {
    if let Some(arch) = extract_property(output, "os.arch") {
        let normalized = match arch.to_ascii_lowercase().as_str() {
            "amd64" | "x86_64" | "x64" => "x64",
            "aarch64" | "arm64" => "arm64",
            "x86" | "i386" | "i486" | "i586" | "i686" => "x86",
            "arm" | "arm32" => "arm",
            _ => arch,
        };
        return normalized.to_string();
    }

    // A generic "64-Bit Server VM" banner does not identify the CPU family.
    let lower = output.to_ascii_lowercase();
    if lower.contains("aarch64") || lower.contains("arm64") {
        "arm64".to_string()
    } else if lower.contains("amd64") || lower.contains("x86_64") {
        "x64".to_string()
    } else if lower.contains("i386") || lower.contains("i686") || lower.contains("x86") {
        "x86".to_string()
    } else {
        "unknown".to_string()
    }
}

fn extract_vendor(output: &str) -> Option<String> {
    let lower = output.to_ascii_lowercase();
    // Most vendor distributions also mention OpenJDK; check specific ones first.
    if lower.contains("adoptium") || lower.contains("temurin") {
        Some("Adoptium".to_string())
    } else if lower.contains("zulu") || lower.contains("azul") {
        Some("Azul Zulu".to_string())
    } else if lower.contains("amazon") || lower.contains("corretto") {
        Some("Amazon Corretto".to_string())
    } else if lower.contains("microsoft") {
        Some("Microsoft".to_string())
    } else if lower.contains("graalvm") {
        Some("GraalVM".to_string())
    } else if lower.contains("oracle") {
        Some("Oracle".to_string())
    } else if lower.contains("openjdk") {
        Some("OpenJDK".to_string())
    } else {
        extract_property(output, "java.vendor").map(str::to_string)
    }
}

/// 扫描系统中常见位置的 Java 安装。
///
/// 结果按「可执行文件路径 + 修改时间」缓存在进程内：设置页每次挂载都会问
/// 一遍，每次都真的去 spawn `java -version` 会明显卡顿。路径或 mtime 变了
/// 自然失效，重装 Java 后不需要手工清缓存。
pub fn discover_java_runtimes() -> Vec<JavaRuntime> {
    let mut runtimes = Vec::new();
    let mut seen = HashSet::new();

    for candidate in discover_java_candidates() {
        // Different installs with the same vendor/major/architecture must remain
        // selectable. Only de-duplicate paths resolving to the same executable.
        let identity = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
        if !seen.insert(identity.clone()) {
            continue;
        }

        if let Some(cached) = cached_runtime(&identity) {
            runtimes.push(cached);
            continue;
        }
        if let Ok(runtime) = JavaRuntime::probe(candidate) {
            remember_runtime(&identity, &runtime);
            runtimes.push(runtime);
        }
    }

    runtimes.sort_by_key(|runtime| std::cmp::Reverse(runtime.major_version));
    runtimes
}

/// 把探测结果写进磁盘缓存，下次启动直接读。
///
/// 进程内缓存解决「同一进程里反复扫」，磁盘缓存解决「每次启动都扫」。
/// 键依然是可执行文件路径 + 修改时间，Java 更新后自然失效。
pub fn save_discovery_cache(path: &Path, runtimes: &[JavaRuntime]) -> std::io::Result<()> {
    let entries: Vec<CachedEntry> = runtimes
        .iter()
        .filter_map(|runtime| {
            let identity = std::fs::canonicalize(&runtime.executable)
                .unwrap_or_else(|_| runtime.executable.clone());
            let stamp = modified_stamp(&identity)?;
            Some(CachedEntry {
                identity: identity.to_string_lossy().into_owned(),
                stamp,
                runtime: runtime.clone(),
            })
        })
        .collect();
    let json = serde_json::to_string(&entries).map_err(std::io::Error::other)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, json)
}

/// 从磁盘缓存读回仍然有效的运行时；任何一项失效就丢弃它。
pub fn load_discovery_cache(path: &Path) -> Vec<JavaRuntime> {
    let Ok(json) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(entries) = serde_json::from_str::<Vec<CachedEntry>>(&json) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for entry in entries {
        let identity = PathBuf::from(&entry.identity);
        // 可执行文件被删掉或换过，就不再认这条缓存。
        if modified_stamp(&identity) != Some(entry.stamp) {
            continue;
        }
        if !seen.insert(entry.identity) {
            continue;
        }
        remember_runtime(&identity, &entry.runtime);
        out.push(entry.runtime);
    }
    out.sort_by_key(|runtime| std::cmp::Reverse(runtime.major_version));
    out
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CachedEntry {
    identity: String,
    stamp: u128,
    runtime: JavaRuntime,
}

type ProbeCache = std::collections::HashMap<PathBuf, (u128, JavaRuntime)>;

/// 进程内缓存：规范路径 → (修改时间, 探测结果)。
fn probe_cache() -> &'static std::sync::Mutex<ProbeCache> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<ProbeCache>> = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// 可执行文件的修改时间（毫秒）；拿不到就不缓存。
fn modified_stamp(path: &Path) -> Option<u128> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis(),
    )
}

fn cached_runtime(identity: &Path) -> Option<JavaRuntime> {
    let stamp = modified_stamp(identity)?;
    let cache = probe_cache().lock().ok()?;
    let (cached_stamp, runtime) = cache.get(identity)?;
    (*cached_stamp == stamp).then(|| runtime.clone())
}

fn remember_runtime(identity: &Path, runtime: &JavaRuntime) {
    let Some(stamp) = modified_stamp(identity) else {
        return;
    };
    if let Ok(mut cache) = probe_cache().lock() {
        cache.insert(identity.to_path_buf(), (stamp, runtime.clone()));
    }
}

#[cfg(target_os = "windows")]
fn discover_java_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 检查 PATH 环境变量
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let java_exe = dir.join("java.exe");
            if java_exe.exists() {
                paths.push(java_exe);
            }
        }
    }

    // 检查 JAVA_HOME
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java_exe = PathBuf::from(java_home).join("bin").join("java.exe");
        if java_exe.exists() {
            paths.push(java_exe);
        }
    }

    // 常见安装位置
    let common_dirs = [
        r"C:\Program Files\Java",
        r"C:\Program Files (x86)\Java",
        r"C:\Program Files\Eclipse Adoptium",
        r"C:\Program Files\Microsoft",
        r"C:\Program Files\Zulu",
        r"C:\Program Files\Amazon Corretto",
    ];

    for base_dir in &common_dirs {
        if let Ok(entries) = std::fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let java_exe = entry.path().join("bin").join("java.exe");
                if java_exe.exists() {
                    paths.push(java_exe);
                }
            }
        }
    }

    // Minecraft 自带的 Java runtime（通常在 .minecraft/runtime）
    if let Ok(appdata) = std::env::var("APPDATA") {
        let minecraft_runtime = PathBuf::from(appdata).join(".minecraft").join("runtime");
        if minecraft_runtime.exists() {
            scan_minecraft_runtime(&minecraft_runtime, &mut paths, "java.exe");
        }
    }

    paths
}

#[cfg(target_os = "linux")]
fn discover_java_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 检查 PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let java_bin = dir.join("java");
            if java_bin.exists() {
                paths.push(java_bin);
            }
        }
    }

    // 检查 JAVA_HOME
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java_bin = PathBuf::from(java_home).join("bin").join("java");
        if java_bin.exists() {
            paths.push(java_bin);
        }
    }

    // 常见安装位置
    let common_dirs = ["/usr/lib/jvm", "/usr/java", "/opt/java", "/opt/jdk"];

    for base_dir in &common_dirs {
        if let Ok(entries) = std::fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let java_bin = entry.path().join("bin").join("java");
                if java_bin.exists() {
                    paths.push(java_bin);
                }
            }
        }
    }

    // Minecraft runtime
    if let Ok(home) = std::env::var("HOME") {
        let minecraft_runtime = PathBuf::from(home).join(".minecraft").join("runtime");
        if minecraft_runtime.exists() {
            scan_minecraft_runtime(&minecraft_runtime, &mut paths, "java");
        }
    }

    paths
}

#[cfg(target_os = "macos")]
fn discover_java_candidates() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    // 检查 PATH
    if let Ok(path_var) = std::env::var("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let java_bin = dir.join("java");
            if java_bin.exists() {
                paths.push(java_bin);
            }
        }
    }

    // 检查 JAVA_HOME
    if let Ok(java_home) = std::env::var("JAVA_HOME") {
        let java_bin = PathBuf::from(java_home).join("bin").join("java");
        if java_bin.exists() {
            paths.push(java_bin);
        }
    }

    // macOS 特有路径
    let common_dirs = [
        "/Library/Java/JavaVirtualMachines",
        "/System/Library/Java/JavaVirtualMachines",
    ];

    for base_dir in &common_dirs {
        if let Ok(entries) = std::fs::read_dir(base_dir) {
            for entry in entries.flatten() {
                let java_bin = entry
                    .path()
                    .join("Contents")
                    .join("Home")
                    .join("bin")
                    .join("java");
                if java_bin.exists() {
                    paths.push(java_bin);
                }
            }
        }
    }

    // Minecraft runtime
    if let Ok(home) = std::env::var("HOME") {
        let minecraft_runtime = PathBuf::from(home)
            .join("Library")
            .join("Application Support")
            .join("minecraft")
            .join("runtime");
        if minecraft_runtime.exists() {
            scan_minecraft_runtime(&minecraft_runtime, &mut paths, "java");
        }
    }

    paths
}

fn scan_minecraft_runtime(base: &Path, paths: &mut Vec<PathBuf>, executable_name: &str) {
    // Mojang commonly installs:
    // runtime/<component>/<platform>/<component>/bin/java(.exe).
    // Bound the depth and avoid symlink directories to prevent cycles.
    let mut pending = vec![(base.to_path_buf(), 0_u8)];
    while let Some((dir, depth)) = pending.pop() {
        let executable = dir.join("bin").join(executable_name);
        if executable.is_file() {
            paths.push(executable);
        }

        if depth >= 4 {
            continue;
        }
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                    pending.push((entry.path(), depth + 1));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 磁盘缓存要能写出去、读回来，并且跳过已失效的条目。
    ///
    /// 这条盯的是「Java 每次都要重扫」：设置页每次挂载都调一次 discover，
    /// 每次都 spawn `java -version` 太慢。缓存键是「规范路径 + mtime」。
    #[test]
    fn discovery_cache_round_trips_and_drops_stale_entries() {
        let dir = std::env::temp_dir().join(format!("aether-java-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        // 用一个真实存在的文件当"可执行文件"，只需要 mtime。
        let fake_java = dir.join("java.exe");
        std::fs::write(&fake_java, b"stub").expect("write stub");
        let cache_file = dir.join("java-runtimes.json");

        let runtime = JavaRuntime {
            executable: fake_java.clone(),
            major_version: 21,
            architecture: "x86_64".to_owned(),
            vendor: Some("Test".to_owned()),
        };
        save_discovery_cache(&cache_file, std::slice::from_ref(&runtime)).expect("save");
        assert!(cache_file.is_file(), "cache file was not written");

        let loaded = load_discovery_cache(&cache_file);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].major_version, 21);

        // 改一下文件内容让 mtime 变化，缓存应当失效。
        std::thread::sleep(std::time::Duration::from_millis(1100));
        std::fs::write(&fake_java, b"stub-v2").expect("rewrite stub");
        let stale = load_discovery_cache(&cache_file);
        assert!(
            stale.is_empty(),
            "stale entry was kept after the file changed"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn parses_java_8_version() {
        let output = r#"java version "1.8.0_391""#;
        assert_eq!(parse_java_major_version(output), Some(8));
    }

    #[test]
    fn parses_java_17_version() {
        let output = r#"openjdk version "17.0.9" 2023-10-17"#;
        assert_eq!(parse_java_major_version(output), Some(17));
    }

    #[test]
    fn parses_java_21_version() {
        let output = r#"java version "21.0.1" 2023-10-17 LTS"#;
        assert_eq!(parse_java_major_version(output), Some(21));
    }

    #[test]
    fn extracts_openjdk_vendor() {
        let output = "openjdk version \"17.0.9\"";
        assert_eq!(extract_vendor(output), Some("OpenJDK".to_string()));
    }

    #[test]
    fn extracts_adoptium_vendor() {
        let output = "Eclipse Temurin Runtime";
        assert_eq!(extract_vendor(output), Some("Adoptium".to_string()));
    }

    #[test]
    fn parses_version_after_unquoted_property_lines() {
        let output = "Property settings:\n    os.arch = amd64\nopenjdk version \"17.0.9\"";
        assert_eq!(parse_java_major_version(output), Some(17));
    }

    #[test]
    fn parses_early_access_version() {
        assert_eq!(
            parse_java_major_version("openjdk version \"25-ea\" 2025-03-18"),
            Some(25)
        );
    }

    #[test]
    fn ignores_unrelated_quoted_lines() {
        assert_eq!(
            parse_java_major_version("setting \"99\"\nnot a java version"),
            None
        );
    }

    #[test]
    fn detects_arm64_from_properties_even_with_64_bit_banner() {
        let output = "Property settings:\n    os.arch = aarch64\nOpenJDK 64-Bit Server VM";
        assert_eq!(extract_architecture(output), "arm64");
    }

    #[test]
    fn does_not_guess_x64_from_generic_64_bit_banner() {
        assert_eq!(extract_architecture("OpenJDK 64-Bit Server VM"), "unknown");
    }

    #[test]
    fn prefers_specific_vendor_to_generic_openjdk() {
        assert_eq!(
            extract_vendor("openjdk version \"21\"\nOpenJDK Runtime Environment Temurin"),
            Some("Adoptium".to_string())
        );
    }

    #[test]
    fn finds_mojang_nested_runtime_layout() {
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "aether-java-runtime-{}-{unique}",
            std::process::id()
        ));
        let name = if cfg!(windows) { "java.exe" } else { "java" };
        let executable = root
            .join("java-runtime-gamma")
            .join("windows-x64")
            .join("java-runtime-gamma")
            .join("bin")
            .join(name);

        std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
        std::fs::File::create(&executable).unwrap();

        let mut paths = Vec::new();
        scan_minecraft_runtime(&root, &mut paths, name);
        let result = paths.contains(&executable);
        std::fs::remove_dir_all(root).unwrap();
        assert!(result, "nested Mojang runtime should be discovered");
    }

    #[test]
    fn discover_returns_sorted_by_version() {
        let runtimes = discover_java_runtimes();
        // 只检查排序，不假设系统一定有 Java
        for i in 1..runtimes.len() {
            assert!(
                runtimes[i - 1].major_version >= runtimes[i].major_version,
                "runtimes should be sorted in descending order"
            );
        }
    }
}
