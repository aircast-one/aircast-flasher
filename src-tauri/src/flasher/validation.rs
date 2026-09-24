use super::format_bytes;

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate the disk path is /dev/disk\d+ (macOS) or /dev/sd[a-z]+ (Linux).
/// This is the dangerous input — it goes to `dd`/`diskutil` as root.
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
        // Accept only `\\.\PhysicalDriveN` (N = digits). This id flows into the
        // elevated write helper, so keep it as tight as the unix device paths.
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

/// Validate that the image path resolves to an existing regular file. The image
/// is only ever read; the native file dialog is the trust boundary for local
/// picks, so we don't restrict its directory (unlike the dangerous disk path).
pub(crate) fn validate_image_file(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
    let canon = path
        .canonicalize()
        .map_err(|e| format!("Image path does not resolve: {e}"))?;
    if !canon.is_file() {
        return Err("Image path is not a file".to_string());
    }
    Ok(canon)
}

/// Validate and normalize a SHA-256 hex digest (lowercase, 64 chars). Rejects
/// HTML error pages and truncated downloads.
pub(crate) fn parse_sha256(raw: &str) -> Result<String, String> {
    if raw.len() != 64 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "Invalid SHA-256 (expected 64 hex chars, got {})",
            raw.len()
        ));
    }
    Ok(raw.to_ascii_lowercase())
}

/// Extract a filename from a URL, stripping query params.
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

/// Hex from the OS CSPRNG, for temp names an attacker must not be able to
/// predict. The PID is guessable and reused; these paths are read by a root
/// process, so guessable is the whole problem.
pub fn randbits() -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("OS randomness");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// The scratch directory the decompressed image, the provision blob and the
/// ready-file handshake live in. All three are consumed by the *elevated*
/// helper, so on Unix the directory is 0700 and must already be ours: a shared
/// `/tmp` otherwise lets any local user swap the image root writes to the card.
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

/// Whether the decompressed image fits, as a check both the pre-elevation and
/// post-decompress paths share.
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

/// Check available disk space at `path`.
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
