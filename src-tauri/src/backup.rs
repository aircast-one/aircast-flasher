use std::path::PathBuf;
use std::time::Duration;

use flasher_ops::backup::{self, Backup, InFlight};
use tauri::{AppHandle, Emitter, Manager, State};

const AUTO_BACKUP_EVERY: Duration = Duration::from_secs(10 * 60);
const BACKUPS_CHANGED: &str = "flasher:backups-changed";

fn dir() -> Result<PathBuf, String> {
    match crate::settings::e2e_dir() {
        Some(dir) => Ok(dir.join("backups")),
        None => flasher_ops::paths::backup_dir(),
    }
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreChoice {
    backup: String,
    same_drone: bool,
}

impl RestoreChoice {
    pub fn file(&self) -> Result<String, String> {
        backup::restore_file(&dir()?, &self.backup, self.same_drone)
    }
}

pub fn back_up_automatically(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut every = tokio::time::interval(AUTO_BACKUP_EVERY);
        loop {
            every.tick().await;
            let Ok(dir) = dir() else { continue };
            let saved = backup::back_up_stale_drones(&dir, app.state::<InFlight>().inner()).await;
            if !saved.is_empty() {
                let _ = app.emit(BACKUPS_CHANGED, ());
            }
        }
    });
}

#[tauri::command]
pub async fn backup_device(in_flight: State<'_, InFlight>, url: String) -> Result<Backup, String> {
    backup::take_once(&dir()?, &url, in_flight.inner()).await
}

#[tauri::command]
pub fn list_backups() -> Result<Vec<Backup>, String> {
    dir().map(|dir| backup::list(&dir))
}

#[tauri::command]
pub fn delete_backup(id: String) -> Result<(), String> {
    backup::delete(&dir()?, &id)
}
