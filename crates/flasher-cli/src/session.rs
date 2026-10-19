use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct Session {
    pub token: String,
    pub email: String,
}

fn path() -> Result<PathBuf, String> {
    flasher_ops::paths::config_dir().map(|dir| dir.join("cli-cloud-session.json"))
}

pub fn load() -> Option<Session> {
    let raw = std::fs::read_to_string(path().ok()?).ok()?;
    serde_json::from_str(&raw).ok()
}

pub fn token() -> String {
    load().map(|session| session.token).unwrap_or_default()
}

pub fn save(session: &Session) -> Result<(), String> {
    let path = path()?;
    let dir = path.parent().ok_or("No config directory")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    let json = serde_json::to_string(session).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&path);
    write_owner_only(&path, &json)
}

pub fn clear() -> Result<(), String> {
    match std::fs::remove_file(path()?) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
        _ => Ok(()),
    }
}

pub fn forget_if_signed_out<T>(result: Result<T, String>) -> Result<T, String> {
    if result
        .as_ref()
        .is_err_and(|e| e == flasher_ops::cloud::SIGNED_OUT)
    {
        clear()?;
    }
    result
}

#[cfg(unix)]
fn write_owner_only(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write as _;
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut file| file.write_all(contents.as_bytes()))
        .map_err(|e| format!("Failed to save the Cloud session: {e}"))
}

#[cfg(not(unix))]
fn write_owner_only(path: &std::path::Path, contents: &str) -> Result<(), String> {
    std::fs::write(path, contents).map_err(|e| format!("Failed to save the Cloud session: {e}"))
}

pub async fn sign_in(on_url: impl FnOnce(&str)) -> Result<String, String> {
    let start = flasher_ops::cloud::sign_in_start().await?;
    on_url(&start.auth_url);
    let _ = open::that(&start.auth_url);
    wait_for_sign_in(&start.token).await
}

pub async fn wait_for_sign_in(token: &str) -> Result<String, String> {
    loop {
        match flasher_ops::cloud::sign_in_poll(token).await? {
            flasher_ops::cloud::SignInStatus::SignedIn {
                email,
                session_token,
            } => {
                save(&Session {
                    token: session_token,
                    email: email.clone(),
                })?;
                return Ok(email);
            }
            flasher_ops::cloud::SignInStatus::Pending => {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    }
}
