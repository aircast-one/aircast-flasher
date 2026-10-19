use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

const SETTINGS_FILE: &str = "settings.json";

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    pub ssid: Option<String>,
    pub hostname: Option<String>,
    pub wifi_password: String,
    pub control_server: String,
    pub authorized_key: Option<String>,
    pub no_wifi: bool,
    pub telemetry: Option<bool>,
    pub install_id: Option<String>,
    pub flashed_hostnames: Vec<String>,
    pub tang_urls: Vec<String>,
    pub tang_threshold: u32,
    pub tang_cloud: bool,
}

const E2E_CONFIG_DIR: &str = "FLASHER_E2E_CONFIG_DIR";

fn config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    match std::env::var_os(E2E_CONFIG_DIR).filter(|_| cfg!(feature = "wdio")) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => app
            .path()
            .app_config_dir()
            .map_err(|e| format!("No config directory: {e}")),
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = config_dir(app)?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config directory: {e}"))?;
    Ok(dir.join(SETTINGS_FILE))
}

fn load_from(path: &Path) -> Settings {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

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
pub fn update_settings(
    app_handle: AppHandle,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<Settings, String> {
    let path = settings_path(&app_handle)?;
    let had_id = load_from(&path).install_id.is_some();
    let next = merge_into(&path, patch)?;
    if next.install_id.is_some() && !had_id {
        crate::telemetry::mark_all_sent(&app_handle);
    }
    Ok(next)
}

fn merge_into(
    path: &Path,
    patch: serde_json::Map<String, serde_json::Value>,
) -> Result<Settings, String> {
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path.with_extension("json.lock"))
        .map_err(|e| format!("Failed to open settings lock: {e}"))?;
    lock.lock()
        .map_err(|e| format!("Failed to lock settings: {e}"))?;
    let current = serde_json::to_value(load_from(path))
        .map_err(|e| format!("Failed to encode settings: {e}"))?;
    let merged = match current {
        serde_json::Value::Object(fields) => {
            serde_json::Value::Object(patch.into_iter().fold(fields, |mut acc, (k, v)| {
                acc.insert(k, v);
                acc
            }))
        }
        other => other,
    };
    let settings: Settings =
        serde_json::from_value(merged).map_err(|e| format!("Invalid settings update: {e}"))?;
    let next = with_install_id(settings);
    save_to(path, &next)?;
    Ok(next)
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
            flashed_hostnames: vec!["aircast-01".into()],
            tang_urls: vec!["http://10.0.0.5".into()],
            tang_threshold: 1,
            tang_cloud: false,
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

    fn patch(json: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
        match json {
            serde_json::Value::Object(map) => map,
            _ => panic!("patch must be an object"),
        }
    }

    #[test]
    fn two_open_windows_saving_different_fields_keep_both() {
        let path = temp_path("merge");
        save_to(&path, &sample()).expect("seed");
        merge_into(
            &path,
            patch(serde_json::json!({ "controlServer": "https://hs.other.example" })),
        )
        .expect("first window");
        merge_into(&path, patch(serde_json::json!({ "noWifi": true }))).expect("second window");
        let saved = load_from(&path);
        assert_eq!(saved.control_server, "https://hs.other.example");
        assert!(saved.no_wifi);
        assert_eq!(saved.wifi_password, sample().wifi_password);
        assert_eq!(saved.ssid, sample().ssid);
    }

    #[test]
    fn a_patch_that_does_not_fit_the_schema_is_refused_and_leaves_the_file_alone() {
        let path = temp_path("bad-patch");
        save_to(&path, &sample()).expect("seed");
        assert!(merge_into(&path, patch(serde_json::json!({ "tangThreshold": "two" }))).is_err());
        assert_eq!(load_from(&path), sample());
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
