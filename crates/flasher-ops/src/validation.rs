use super::format_bytes;
use flasher_core::{AccessConfig, ProvisionConfig, MIN_FIGHT_PASSWORD};

pub(crate) fn validate_disk_path(path: &str) -> Result<(), String> {
    let valid = if cfg!(target_os = "macos") {
        path.starts_with("/dev/disk")
            && path["/dev/disk".len()..]
                .chars()
                .all(|c| c.is_ascii_digit())
            && path.len() > "/dev/disk".len()
    } else if cfg!(target_os = "linux") {
        path.starts_with("/dev/sd")
            && path["/dev/sd".len()..]
                .chars()
                .all(|c| c.is_ascii_lowercase())
            && path.len() > "/dev/sd".len()
    } else if cfg!(target_os = "windows") {
        const PREFIX: &str = r"\\.\PhysicalDrive";
        path.starts_with(PREFIX)
            && path[PREFIX.len()..].chars().all(|c| c.is_ascii_digit())
            && path.len() > PREFIX.len()
    } else {
        false
    };
    if !valid {
        return Err(format!("Invalid disk path: {path}"));
    }
    Ok(())
}

pub(crate) fn validate_fight_access(config: &ProvisionConfig) -> Result<(), String> {
    let holds = config.access.as_ref().is_some_and(AccessConfig::holds_in_fight);
    if config.fight_mode && !holds {
        return Err(format!(
            "Fight mode needs an SSH key, a password of at least {MIN_FIGHT_PASSWORD} characters, or SSH turned off — the image's default login must not stay on."
        ));
    }
    Ok(())
}

pub(crate) fn validate_image_file(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let canon = path
        .canonicalize()
        .map_err(|e| format!("Image path does not resolve: {e}"))?;
    if !canon.is_file() {
        return Err("Image path is not a file".to_string());
    }
    Ok(canon)
}

pub(crate) fn parse_sha256(raw: &str) -> Result<String, String> {
    if raw.len() != 64 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "Invalid SHA-256 (expected 64 hex chars, got {})",
            raw.len()
        ));
    }
    Ok(raw.to_ascii_lowercase())
}

pub(crate) fn filename_from_url(url_str: &str) -> String {
    url::Url::parse(url_str)
        .ok()
        .and_then(|u| {
            u.path_segments()
                .and_then(|mut s| s.next_back())
                .map(|s| s.to_string())
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "image.img.xz".to_string())
}

pub fn random_hex(len: usize) -> String {
    let mut bytes = vec![0u8; len];
    getrandom::fill(&mut bytes).expect("OS randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn randbits() -> String {
    random_hex(8)
}

pub(crate) fn private_temp_dir() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join("aircast-flasher");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let meta =
            std::fs::symlink_metadata(&dir).map_err(|e| format!("Failed to stat temp dir: {e}"))?;
        if !meta.is_dir() {
            return Err(format!("{} is not a directory", dir.display()));
        }
        if meta.uid() != unsafe { libc::geteuid() } {
            return Err(format!(
                "{} is owned by another user — refusing to stage a root-written image there",
                dir.display()
            ));
        }
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| format!("Failed to lock down temp dir: {e}"))?;
    }

    Ok(dir)
}

pub(crate) fn fits_on_card(
    image_bytes: u64,
    card_size: u64,
    card_label: &str,
) -> Result<(), String> {
    if card_size > 0 && image_bytes > card_size {
        return Err(format!(
            "Card too small: the image needs {} but {card_label} holds {} — use a larger card.",
            format_bytes(image_bytes),
            format_bytes(card_size)
        ));
    }
    Ok(())
}

pub(crate) fn check_available_space(path: &std::path::Path, required: u64) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        let c_path = CString::new(path.to_string_lossy().as_bytes())
            .map_err(|e| format!("Invalid path: {e}"))?;
        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) != 0 {
                return Err("Failed to check available disk space".to_string());
            }
            let available = (stat.f_bavail as u64) * (stat.f_frsize as u64);
            if available < required {
                return Err(format!(
                    "Insufficient disk space: {:.1} GB available, {:.1} GB required",
                    available as f64 / 1e9,
                    required as f64 / 1e9,
                ));
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = (path, required);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use flasher_core::{InitFormat, SshMode};

    fn config(access: Option<AccessConfig>, fight_mode: bool) -> ProvisionConfig {
        ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: None,
            access,
            encryption: None,
            fight_mode,
            init_format: InitFormat::CloudInit,
        }
    }

    fn ssh(ssh: SshMode, key: Option<&str>) -> Option<AccessConfig> {
        Some(AccessConfig {
            ssh,
            authorized_key: key.map(str::to_string),
            password: None,
        })
    }

    fn password(pw: &str) -> Option<AccessConfig> {
        Some(AccessConfig {
            ssh: SshMode::Password,
            authorized_key: None,
            password: Some(pw.into()),
        })
    }

    #[test]
    fn fight_mode_takes_a_key_or_a_long_password() {
        let key = Some("ssh-ed25519 AAAA me");
        assert!(validate_fight_access(&config(ssh(SshMode::KeyOnly, key), true)).is_ok());
        assert!(validate_fight_access(&config(ssh(SshMode::Disabled, None), true)).is_ok());
        assert!(validate_fight_access(&config(password("twelve-chars"), true)).is_ok());
        assert!(validate_fight_access(&config(password("eleven-char"), true)).is_err());
        assert!(
            validate_fight_access(&config(password("  eleven-char  "), true)).is_err(),
            "padding doesn't count toward the length"
        );
        assert!(validate_fight_access(&config(ssh(SshMode::Password, None), true)).is_err());
        assert!(
            validate_fight_access(&config(ssh(SshMode::KeyOnly, Some("  ")), true)).is_err(),
            "a blank key would leave the image's password login on"
        );
        assert!(
            validate_fight_access(&config(None, true)).is_err(),
            "no access settings keep the image's password login"
        );
        assert!(validate_fight_access(&config(None, false)).is_ok());
        assert!(validate_fight_access(&config(password("short"), false)).is_ok());
    }
}
