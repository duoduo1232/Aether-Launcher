//! Instance listing and creation commands.
//!
//! The store root is resolved by the backend from the application data
//! directory, so the frontend never chooses a path on disk. Listing tolerates
//! individual broken records and reports why they were skipped; creation only
//! accepts a display name and a version identifier.

use std::path::PathBuf;

use aether_core::{GameInstance, InstanceState};
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_dialog::DialogExt;

/// One entry of the instance list as the frontend sees it.
///
/// Every type that crosses the IPC boundary must be public: Tauri generates a
/// wrapper whose signature mentions it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceSummary {
    id: String,
    name: String,
    version: String,
    state: &'static str,
}

impl InstanceSummary {
    pub(crate) fn from_instance(instance: &GameInstance) -> Self {
        Self {
            id: instance.id.to_string(),
            name: instance.name.clone(),
            version: instance.version.as_str().to_owned(),
            state: instance_state_label(&instance.state),
        }
    }
}

/// A record that the listing pass refused to load, with a stable category
/// instead of a raw file system path.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedInstance {
    id: String,
    reason: &'static str,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstanceListing {
    instances: Vec<InstanceSummary>,
    skipped: Vec<SkippedInstance>,
}

fn instance_state_label(state: &InstanceState) -> &'static str {
    match state {
        InstanceState::Ready => "ready",
        InstanceState::Incomplete => "incomplete",
        InstanceState::Running => "running",
        InstanceState::Failed => "failed",
    }
}

/// Resolve the application-owned instance store root and make sure the
/// `instances/` directory exists. The path comes from Tauri, never from IPC.
fn instance_store_root(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let base = app
        .path()
        .app_data_dir()
        .map_err(|error| format!("application data directory is unavailable: {error}"))?;

    std::fs::create_dir_all(base.join("instances"))
        .map_err(|error| format!("cannot prepare the instance store: {error}"))?;

    Ok(base)
}

#[tauri::command]
pub(crate) async fn list_instances(app: tauri::AppHandle) -> Result<InstanceListing, String> {
    let root = instance_store_root(&app)?;

    tauri::async_runtime::spawn_blocking(move || match aether_storage::list_instances(&root) {
        Ok(listing) => Ok(InstanceListing {
            instances: listing
                .instances
                .iter()
                .map(InstanceSummary::from_instance)
                .collect(),
            skipped: listing
                .skipped
                .iter()
                .map(|skipped| SkippedInstance {
                    id: skipped.id.clone(),
                    reason: skipped.reason,
                })
                .collect(),
        }),
        Err(error) => Err(format!("cannot list instances: {error}")),
    })
    .await
    .map_err(|error| format!("instance listing task failed: {error}"))?
}

/// 新建实例。游戏目录由**后端**弹出选择框让用户挑，前端从不传路径。
///
/// 用户取消选择时返回 `Ok(None)`：这不是错误，不需要提示。
#[tauri::command]
pub(crate) async fn create_instance(
    app: tauri::AppHandle,
    name: String,
    version: String,
) -> Result<Option<InstanceSummary>, String> {
    let root = instance_store_root(&app)?;
    let Some(game_directory) = pick_game_directory(&app).await? else {
        return Ok(None);
    };

    tauri::async_runtime::spawn_blocking(move || {
        aether_storage::create_instance(&root, &name, &version, &game_directory)
            .map(|instance| InstanceSummary::from_instance(&instance))
            .map_err(|error| format!("cannot create instance: {error}"))
    })
    .await
    .map_err(|error| format!("instance creation task failed: {error}"))?
    .map(Some)
}

/// 让用户选择一个 `.minecraft` 作为游戏目录；取消时返回 `None`。
async fn pick_game_directory(app: &tauri::AppHandle) -> Result<Option<PathBuf>, String> {
    let (sender, receiver) = std::sync::mpsc::channel();
    app.dialog()
        .file()
        .set_title("选择游戏目录（.minecraft）")
        .pick_folder(move |picked| {
            let _ = sender.send(picked);
        });
    receiver
        .recv()
        .map_err(|_| "the folder picker closed unexpectedly".to_owned())?
        .map(|path| {
            path.into_path()
                .map_err(|error| format!("the chosen folder is not usable: {error}"))
        })
        .transpose()
}

/// 给所有还没设置游戏目录的实例指定一个 `.minecraft`。
///
/// 这是老实例（`game_directory` 字段加入之前创建的）唯一的补救方式：Aether 不
/// 从别的启动器读配置，也不猜路径，所以由用户在后端点一次目录选择框，把结果
/// 写进每一条需要修的记录。
///
/// 写入前会把 `instance.json` 备份到 `<store>/instances/.backup-<时间戳>/`；
/// 已经设置过游戏目录的实例不动。
#[tauri::command]
pub(crate) async fn set_game_directory_for_all_instances(
    app: tauri::AppHandle,
) -> Result<GameDirectoryReport, String> {
    let root = instance_store_root(&app)?;
    let Some(game_directory) = pick_game_directory(&app).await? else {
        return Ok(GameDirectoryReport {
            updated: 0,
            skipped: 0,
            failed: Vec::new(),
            cancelled: true,
        });
    };

    tauri::async_runtime::spawn_blocking(move || {
        aether_storage::apply_game_directory(&root, &game_directory)
            .map(Into::into)
            .map_err(|error| format!("cannot set the game directory: {error}"))
    })
    .await
    .map_err(|error| format!("game directory task failed: {error}"))?
}

/// 一次批量设置的结果。`cancelled` 为真时其余字段没有意义。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GameDirectoryReport {
    updated: usize,
    skipped: usize,
    failed: Vec<String>,
    cancelled: bool,
}

impl From<aether_storage::GameDirectoryOutcome> for GameDirectoryReport {
    fn from(outcome: aether_storage::GameDirectoryOutcome) -> Self {
        Self {
            updated: outcome.updated,
            skipped: outcome.skipped,
            failed: outcome.failed,
            cancelled: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> GameInstance {
        GameInstance {
            id: aether_core::InstanceId::new(),
            name: "Survival".to_owned(),
            version: aether_core::VersionId::parse("1.21.1").unwrap(),
            root: PathBuf::from("instances"),
            game_directory: None,
            state: InstanceState::Ready,
        }
    }

    #[test]
    fn summary_fields_are_camel_case_and_stable() {
        let value = serde_json::to_value(InstanceSummary::from_instance(&sample())).unwrap();

        assert_eq!(value["name"], "Survival");
        assert_eq!(value["version"], "1.21.1");
        assert_eq!(value["state"], "ready");
        assert!(value.get("memory_mib").is_none());
        assert!(value.get("root").is_none());
    }

    #[test]
    fn every_instance_state_has_a_lowercase_label() {
        for (state, label) in [
            (InstanceState::Ready, "ready"),
            (InstanceState::Incomplete, "incomplete"),
            (InstanceState::Running, "running"),
            (InstanceState::Failed, "failed"),
        ] {
            assert_eq!(instance_state_label(&state), label);
        }
    }
}
