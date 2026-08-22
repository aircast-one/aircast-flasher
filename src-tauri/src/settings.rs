use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "settings.json";

/// Operator settings that survive restarts. `None` means "never set", which is
/// what lets a cleared field stay cleared instead of snapping back to the
/// detected network or a generated hostname. The pre-auth key is deliberately
/// absent — it is a short-lived secret and stays in memory only.
#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub ssid: Option<String>,
    pub hostname: Option<String>,
    pub wifi_password: String,
    pub control_server: String,
    /// `None` = never set, so a single detected key prefills the field;
    /// `Some("")` = the operator cleared it and must not be re-prefilled.
    pub authorized_key: Option<String>,
    /// The operator flashes wired or cellular devices: WiFi is skipped on
    /// purpose. Persisted like the rest, so a fleet of Ethernet boards does not
    /// re-raise "enter a network name" on every launch.
    pub no_wifi: bool,
    /// `None` = never asked, which is what raises the consent prompt exactly
    /// once. Diagnostics leave the machine only on `Some(true)`.
    pub telemetry: Option<bool>,
    /// Random per-install id, minted when diagnostics are turned on and never
    /// derived from anything about the machine or its operator.
    pub install_id: Option<String>,
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("No config directory: {e}"))?;
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create config directory: {e}"))?;
    Ok(dir.join(SETTINGS_FILE))
}

/// Unreadable or corrupt settings are not an error: a first run has no file,
/// and a truncated one must not brick the wizard. Either way the operator gets
/// defaults and the next save rewrites it.
fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

/// Owner-only (0600 on Unix): this file holds the WiFi passphrase.
fn save_to(path: &Path, settings: &Settings) -> Result<(), String> {
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| format!("Failed to encode settings: {e}"))?;

    let tmp = path.with_extension("json.tmp");
    write_owner_only(&tmp, &json)?;
    std::fs::rename(&tmp, path).map_err(|e| format!("Failed to save settings: {e}"))
}

fn write_owner_only(path: &Path, contents: &str) -> Result<(), String> {
    use std::io::Write as _;
    let _ = std::fs::remove_file(path);

    let mut file = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        }
        #[cfg(not(unix))]
        {
            std::fs::File::create(path)
        }
    }
    .map_err(|e| format!("Failed to open settings for writing: {e}"))?;

    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write settings: {e}"))
}

/// Turning diagnostics on mints the install id; turning them off drops it, so
/// an opt-out leaves nothing behind to correlate a later opt-in against.
fn with_install_id(settings: Settings) -> Settings {
    match settings.telemetry {
        Some(true) => Settings {
            install_id: settings
                .install_id
                .filter(|id| !id.is_empty())
                .or_else(|| Some(crate::flasher::randbits())),
            ..settings
        },
        _ => Settings {
            install_id: None,
            ..settings
        },
    }
}

/// The id events are attributed to, or `None` when the operator has not opted
/// in — the single gate every shipped event passes through.
pub fn telemetry_id(app: &AppHandle) -> Option<String> {
    let settings = load_from(&settings_path(app).ok()?);
    settings.telemetry.unwrap_or(false).then_some(())?;
    settings.install_id.filter(|id| !id.is_empty())
}

#[tauri::command]
pub fn read_settings(app_handle: AppHandle) -> Result<Settings, String> {
    Ok(load_from(&settings_path(&app_handle)?))
}

#[tauri::command]
pub fn write_settings(app_handle: AppHandle, settings: Settings) -> Result<(), String> {
    let path = settings_path(&app_handle)?;
    let next = with_install_id(settings);
    if next.install_id.is_some() && load_from(&path).install_id.is_none() {
        crate::telemetry::mark_all_sent(&app_handle);
    }
    save_to(&path, &next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aircast-settings-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(SETTINGS_FILE)
    }

    fn sample() -> Settings {
        Settings {
            ssid: Some("field-net".into()),
            hostname: Some("falcon-01".into()),
            wifi_password: "passphrase".into(),
            control_server: "https://hs.example.com".into(),
            authorized_key: Some("ssh-ed25519 AAAA pavliha@mac".into()),
            no_wifi: false,
            telemetry: None,
            install_id: None,
        }
    }

    #[test]
    fn settings_round_trip() {
        let path = temp_path("roundtrip");
        save_to(&path, &sample()).expect("save");
        assert_eq!(load_from(&path), sample());
    }

    #[test]
    fn a_cleared_field_stays_cleared_and_an_unset_one_stays_unset() {
        let path = temp_path("cleared");
        let settings = Settings {
            ssid: Some(String::new()),
            hostname: None,
            ..Settings::default()
        };
        save_to(&path, &settings).expect("save");

        let loaded = load_from(&path);
        assert_eq!(loaded.ssid.as_deref(), Some(""), "cleared, not unset");
        assert_eq!(loaded.hostname, None, "unset, not cleared");
    }

    #[test]
    fn a_missing_file_reads_as_defaults() {
        let path = temp_path("missing").with_file_name("nope.json");
        assert_eq!(load_from(&path), Settings::default());
    }

    #[test]
    fn corrupt_settings_read_as_defaults_instead_of_failing() {
        let path = temp_path("corrupt");
        std::fs::write(&path, "{ this is not json").expect("seed");
        assert_eq!(load_from(&path), Settings::default());
    }

    #[test]
    fn unknown_fields_are_ignored_so_an_older_build_can_read_a_newer_file() {
        let path = temp_path("forward");
        std::fs::write(&path, r#"{"hostname":"falcon-01","futureField":42}"#).expect("seed");
        assert_eq!(load_from(&path).hostname.as_deref(), Some("falcon-01"));
    }

    #[test]
    fn the_wifi_passphrase_is_never_world_readable() {
        let path = temp_path("perms");
        save_to(&path, &sample()).expect("save");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(&path).expect("stat").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "settings must be owner-only");
        }
    }

    #[test]
    fn opting_in_mints_an_install_id_and_keeps_it_across_saves() {
        let opted_in = with_install_id(Settings {
            telemetry: Some(true),
            ..sample()
        });
        let id = opted_in.install_id.clone().expect("id minted on opt-in");
        assert!(!id.is_empty());
        assert_eq!(with_install_id(opted_in).install_id, Some(id));
    }

    #[test]
    fn opting_out_drops_the_install_id() {
        let opted_in = with_install_id(Settings {
            telemetry: Some(true),
            ..sample()
        });
        for answer in [Some(false), None] {
            let settings = with_install_id(Settings {
                telemetry: answer,
                ..opted_in.clone()
            });
            assert_eq!(settings.install_id, None, "answer {answer:?} kept an id");
        }
    }

    #[test]
    fn two_installs_do_not_share_an_id() {
        let mint = || {
            with_install_id(Settings {
                telemetry: Some(true),
                ..Settings::default()
            })
            .install_id
        };
        assert_ne!(mint(), mint());
    }

    #[test]
    fn saving_twice_overwrites_rather_than_failing() {
        let path = temp_path("overwrite");
        save_to(&path, &sample()).expect("first save");
        let second = Settings {
            hostname: Some("falcon-02".into()),
            ..sample()
        };
        save_to(&path, &second).expect("second save");
        assert_eq!(load_from(&path), second);
        assert!(
            !path.with_extension("json.tmp").exists(),
            "no temp file left behind"
        );
    }
}
