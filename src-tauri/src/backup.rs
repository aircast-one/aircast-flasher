use std::path::PathBuf;

use flasher_ops::backup::{self, Backup};

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

#[tauri::command]
pub async fn backup_device(url: String) -> Result<Backup, String> {
    backup::take(&dir()?, &url).await
}

#[tauri::command]
pub fn list_backups() -> Result<Vec<Backup>, String> {
    dir().map(|dir| backup::list(&dir))
}

#[tauri::command]
pub fn delete_backup(id: String) -> Result<(), String> {
    backup::delete(&dir()?, &id)
}
