mod commands;

use std::path::PathBuf;

use aether_runtime::JavaRuntime;
use serde::Serialize;
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ApplicationInfo {
    name: &'static str,
    version: &'static str,
    core_status: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct JavaRuntimeInfo {
    executable: PathBuf,
    major_version: u16,
    architecture: String,
    vendor: Option<String>,
}

impl From<JavaRuntime> for JavaRuntimeInfo {
    fn from(runtime: JavaRuntime) -> Self {
        Self {
            executable: runtime.executable,
            major_version: runtime.major_version,
            architecture: runtime.architecture,
            vendor: runtime.vendor,
        }
    }
}

#[tauri::command]
fn application_info() -> ApplicationInfo {
    ApplicationInfo {
        name: "Aether Launcher",
        version: env!("CARGO_PKG_VERSION"),
        core_status: "foundation",
    }
}

/// A read-only IPC command: enumerate detected Java installations without
/// blocking the Tauri main thread. No caller-provided executable is run.
#[tauri::command]
async fn list_java_runtimes(
    app: tauri::AppHandle,
    refresh: Option<bool>,
) -> Result<Vec<JavaRuntimeInfo>, String> {
    let refresh = refresh.unwrap_or(false);
    // 磁盘缓存：Java 很少变，没必要每次启动都 spawn 一遍 `java -version`。
    let cache_path = app
        .path()
        .app_data_dir()
        .ok()
        .map(|dir| dir.join("java-runtimes.json"));

    tauri::async_runtime::spawn_blocking(move || {
        if !refresh && let Some(path) = &cache_path {
            let cached = aether_runtime::load_discovery_cache(path);
            if !cached.is_empty() {
                return cached;
            }
        }
        let fresh = aether_runtime::discover_java_runtimes();
        if let Some(path) = &cache_path {
            let _ = aether_runtime::save_discovery_cache(path, &fresh);
        }
        fresh
    })
    .await
    .map(|runtimes| runtimes.into_iter().map(JavaRuntimeInfo::from).collect())
    .map_err(|error| format!("Java discovery task failed: {error}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::launch::LaunchState::default())
        .manage(commands::import::ImportState::default())
        .invoke_handler(tauri::generate_handler![
            application_info,
            list_java_runtimes,
            commands::catalog::list_minecraft_versions,
            commands::vanilla::cache_vanilla_version_manifest,
            commands::install::install_vanilla_client_libraries,
            commands::instances::list_instances,
            commands::instances::create_instance,
            commands::instances::set_game_directory_for_all_instances,
            commands::instances::delete_instance,
            commands::import::pick_import_folder,
            commands::import::detect_launcher_installations,
            commands::import::pick_launcher_folder,
            commands::import::import_versions,
            commands::launch::activate_installed_instance,
            commands::launch::start_installed_instance,
            commands::launch::stop_installed_instance,
            commands::schematics::list_schematics,
            commands::schematics::read_schematic,
            commands::schematics::import_schematic,
            commands::schematics::import_schematic_folder,
            commands::schematics::delete_schematic,
            commands::schematics::reveal_schematic,
            commands::schematics::paste_schematic,
            commands::schematics::open_schematic_window,
            commands::schematics::asset_pack_commands::list_asset_pack_candidates,
            commands::schematics::asset_pack_commands::load_asset_pack,
            commands::schematics::asset_pack_commands::asset_pack_status,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_runtime_response_uses_camel_case() {
        let runtime = JavaRuntime {
            executable: PathBuf::from("java"),
            major_version: 21,
            architecture: "arm64".to_string(),
            vendor: Some("Adoptium".to_string()),
        };
        let value = serde_json::to_value(JavaRuntimeInfo::from(runtime)).unwrap();

        assert_eq!(value["executable"], "java");
        assert_eq!(value["majorVersion"], 21);
        assert_eq!(value["architecture"], "arm64");
        assert_eq!(value["vendor"], "Adoptium");
        assert!(value.get("major_version").is_none());
    }

    #[test]
    fn java_runtime_response_preserves_missing_vendor() {
        let runtime = JavaRuntime {
            executable: PathBuf::from("java"),
            major_version: 17,
            architecture: "unknown".to_string(),
            vendor: None,
        };
        let value = serde_json::to_value(JavaRuntimeInfo::from(runtime)).unwrap();
        assert!(value["vendor"].is_null());
    }
}
