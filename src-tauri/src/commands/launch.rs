//! Offline launch of an already installed official Minecraft game.
//! IPC accepts typed instance IDs and an offline player name, never executable
//! paths, JVM arguments, URLs or filesystem roots from the frontend.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Mutex;

use aether_auth::offline::offline_uuid;
use aether_core::{InstanceId, InstanceState};
use aether_download::assets::verify_installed_assets;
use aether_download::verify::verify_local_artifact;
use aether_download::{ArtifactSource, DownloadArtifact};
use aether_launch::{InstanceLaunchCoordinator, InstanceProcessStatus};
use aether_manifest::load_resolved_version_manifest;
use aether_manifest::rules::Environment;
use aether_storage::{InstanceStore, set_instance_state};
use tauri::Manager;

/// Owned for the lifetime of the Tauri application, not per IPC invocation.
/// The registry prevents a duplicate launch from losing an earlier child handle.
///
/// A coordinator is bound to one game directory, and two instances may point at
/// different `.minecraft` folders, so they are kept per canonical game directory.
#[derive(Default)]
pub struct LaunchState {
    coordinators: Mutex<BTreeMap<PathBuf, InstanceLaunchCoordinator>>,
}

fn parse_id(id: &str) -> Result<InstanceId, String> {
    InstanceId::parse(id).map_err(|_| "invalid instance ID".to_owned())
}

fn instance_store_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("application storage unavailable: {error}"))?;
    std::fs::create_dir_all(base.join("instances"))
        .map_err(|error| format!("instance storage unavailable: {error}"))?;
    Ok(base)
}

/// The game directory of one instance: the real `.minecraft` it plays in.
///
/// It is resolved from the instance record, never from IPC, so the frontend cannot
/// redirect a launch at an arbitrary folder. Old records without the field fall
/// back to looking up the `.minecraft` that actually contains the instance's
/// version — the same rule schematic discovery uses, so both agree.
fn instance_game_directory(instance: &aether_core::GameInstance) -> Result<PathBuf, String> {
    let path = super::schematics::resolve_game_directory(instance);
    if !path.join("versions").is_dir() {
        return Err(format!(
            "this instance's game directory has no versions folder: {}",
            path.display()
        ));
    }
    std::fs::canonicalize(&path)
        .map_err(|error| format!("cannot open the instance game directory: {error}"))
}

pub(super) fn current_environment() -> Environment {
    Environment {
        os_name: if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "osx"
        } else {
            "linux"
        }
        .to_owned(),
        os_arch: std::env::consts::ARCH.to_owned(),
        os_version: String::new(),
        features: Default::default(),
    }
}

/// Verify the client JAR and assets before considering a previously created
/// instance installed. Library/native verification happens again when launching.
#[tauri::command]
pub async fn activate_installed_instance(app: tauri::AppHandle, id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let id = parse_id(&id)?;
        let store_root = instance_store_root(&app)?;
        let instance = InstanceStore::open(&store_root)
            .map_err(|error| error.to_string())?
            .load(id)
            .map_err(|error| error.to_string())?;
        let game_root = instance_game_directory(&instance)?;
        let resolved = load_resolved_version_manifest(&game_root, instance.version.as_str())
            .map_err(|error| error.to_string())?;
        let client_version = resolved
            .client_jar_version
            .ok_or_else(|| "installed version has no verified client JAR metadata".to_owned())?;
        let client = resolved
            .manifest
            .downloads
            .as_ref()
            .and_then(|download| download.client.as_ref())
            .ok_or_else(|| "installed version has no client metadata".to_owned())?;
        let artifact = DownloadArtifact {
            destination: PathBuf::from("versions")
                .join(&client_version)
                .join(format!("{client_version}.jar")),
            size: client.size,
            sha1: client.sha1.clone(),
            sources: vec![ArtifactSource {
                provider: "mojang".to_owned(),
                url: client.url.clone(),
            }],
        };
        verify_local_artifact(&game_root, &artifact).map_err(|error| error.to_string())?;
        if let Some(index) = &resolved.manifest.asset_index {
            verify_installed_assets(&game_root, &index.id, &index.sha1, index.size)
                .map_err(|error| error.to_string())?;
        }
        if instance.state != InstanceState::Ready {
            set_instance_state(&instance.root, &instance, InstanceState::Ready)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|error| format!("installation validation task failed: {error}"))?
}

/// Launch only a verified managed instance, using backend-discovered Java.
/// This MVP supports offline single-player only; Microsoft auth is not faked.
#[tauri::command]
pub async fn start_installed_instance(
    app: tauri::AppHandle,
    id: String,
    offline_name: String,
    memory_mib: u32,
) -> Result<InstanceProcessStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let id = parse_id(&id)?;
        let uuid = offline_uuid(&offline_name).map_err(|error| error.to_string())?;
        let root = instance_store_root(&app)?;
        let instance = InstanceStore::open(&root)
            .map_err(|error| error.to_string())?
            .load(id)
            .map_err(|error| error.to_string())?;
        let game = instance_game_directory(&instance)?;
        let mut vars = BTreeMap::new();
        vars.insert("auth_player_name".to_owned(), offline_name);
        vars.insert("auth_uuid".to_owned(), uuid.simple().to_string());
        vars.insert("auth_access_token".to_owned(), "0".to_owned());
        vars.insert("user_type".to_owned(), "legacy".to_owned());
        vars.insert("version_type".to_owned(), "release".to_owned());
        vars.insert("clientid".to_owned(), String::new());
        vars.insert("xuid".to_owned(), String::new());
        vars.insert("launcher_name".to_owned(), "Aether".to_owned());
        vars.insert(
            "launcher_version".to_owned(),
            env!("CARGO_PKG_VERSION").to_owned(),
        );

        let runtimes = aether_runtime::discover_java_runtimes();
        let state = app.state::<LaunchState>();
        let mut guard = state
            .coordinators
            .lock()
            .map_err(|_| "game process registry is unavailable".to_owned())?;
        if !guard.contains_key(&game) {
            let coordinator =
                InstanceLaunchCoordinator::open(&root, &game).map_err(|error| error.to_string())?;
            guard.insert(game.clone(), coordinator);
        }
        guard
            .get_mut(&game)
            .ok_or_else(|| "game launcher unavailable".to_owned())?
            .start(
                id,
                memory_mib,
                &runtimes,
                &current_environment(),
                &vars,
                &[],
            )
            .map_err(|error| error.to_string())
    })
    .await
    .map_err(|error| format!("game launch task failed: {error}"))?
}

/// Stop only a game process launched by this application session.
#[tauri::command]
pub async fn stop_installed_instance(
    app: tauri::AppHandle,
    id: String,
) -> Result<InstanceProcessStatus, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let id = parse_id(&id)?;
        let state = app.state::<LaunchState>();
        let mut guard = state
            .coordinators
            .lock()
            .map_err(|_| "game process registry is unavailable".to_owned())?;
        // 哪个游戏目录启动的就在哪个里停：同一进程会话里可能有多个游戏目录。
        for coordinator in guard.values_mut() {
            if coordinator.status(id).is_ok() {
                return coordinator.stop(id).map_err(|error| error.to_string());
            }
        }
        Err("no game was started in this session".to_owned())
    })
    .await
    .map_err(|error| format!("game stop task failed: {error}"))?
}
