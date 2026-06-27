//! Flasher commands. Mirrors the proven aircast-web flasher: list releases,
//! enumerate removable block devices, stream-download with checksum verify,
//! decompress if needed, then run the open-once write → verify → customize
//! engine inside one elevated helper process (Touch ID on macOS / UAC on
//! Windows / pkexec on Linux). Cancellable via a shared flag.

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager};

use flasher_core::{ProvisionConfig, WifiConfig};

const PROGRESS_THROTTLE_MS: u128 = 100;
const MIN_TEMP_SPACE_BYTES: u64 = 4_500_000_000; // ~4.5 GB

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Shared cancel flag for the active download/flash. `cancel_flash` flips it;
/// long-running loops poll it.
pub struct FlasherState {
    pub cancel: Arc<AtomicBool>,
}

impl FlasherState {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

// ---------------------------------------------------------------------------
// Types — releases.json
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageInfo {
    pub filename: String,
    pub extension: String,
    pub size: u64,
    pub download_url: String,
    pub checksum_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub prerelease: bool,
    pub created_at: String,
    pub image: ImageInfo,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleasesResponse {
    pub releases: Vec<Release>,
}

// ---------------------------------------------------------------------------
// Types — block devices & progress
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockDevice {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub size_human: String,
    pub removable: bool,
    pub mounted: bool,
    pub mount_points: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadProgress {
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub percent: f64,
    pub speed_bps: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FlashProgress {
    pub phase: FlashPhase,
    pub bytes_processed: u64,
    pub total_bytes: u64,
    pub percent: f64,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FlashPhase {
    Decompressing,
    Writing,
    Verifying,
    Customizing,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadResult {
    pub image_path: String,
    pub checksum: String,
    pub cached: bool,
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// Validate the disk path is /dev/disk\d+ (macOS) or /dev/sd[a-z]+ (Linux).
/// This is the dangerous input — it goes to `dd`/`diskutil` as root.
fn validate_disk_path(path: &str) -> Result<(), String> {
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
fn validate_image_file(path: &std::path::Path) -> Result<std::path::PathBuf, String> {
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
fn parse_sha256(raw: &str) -> Result<String, String> {
    if raw.len() != 64 || !raw.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!(
            "Invalid SHA-256 (expected 64 hex chars, got {})",
            raw.len()
        ));
    }
    Ok(raw.to_ascii_lowercase())
}

/// Extract a filename from a URL, stripping query params.
fn filename_from_url(url_str: &str) -> String {
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

/// Check available disk space at `path`.
fn check_available_space(path: &std::path::Path, required: u64) -> Result<(), String> {
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

// ---------------------------------------------------------------------------
// Command: list_releases
// ---------------------------------------------------------------------------

/// Channels in preference order, used when no explicit channel is requested.
/// `stable` is preferred but may have no published release yet, so we fall
/// through to `staging` then `development`.
const CHANNEL_FALLBACK: &[&str] = &["stable", "staging", "development"];

fn channel_host(channel: &str) -> Result<&'static str, String> {
    match channel {
        "stable" => Ok("downloads.aircast.one"),
        "development" => Ok("downloads.dev.aircast.one"),
        "staging" => Ok("downloads.stage.aircast.one"),
        other => Err(format!(
            "Unknown channel: {other} (expected stable|development|staging)"
        )),
    }
}

async fn fetch_releases(channel: &str) -> Result<ReleasesResponse, String> {
    let host = channel_host(channel)?;
    let url = format!("https://{host}/lite/releases.json");
    let response = reqwest::get(&url)
        .await
        .map_err(|e| format!("Failed to fetch releases: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<ReleasesResponse>()
        .await
        .map_err(|e| format!("Failed to parse releases: {e}"))
}

#[tauri::command]
pub async fn list_releases(channel: Option<String>) -> Result<ReleasesResponse, String> {
    // Explicit channel: use only that one.
    if let Some(ch) = channel {
        return fetch_releases(&ch).await;
    }

    // No channel: prefer stable, fall back to staging/development so the app
    // works before a stable image is cut, and auto-upgrades once it is.
    let mut last_err = "no channels tried".to_string();
    for ch in CHANNEL_FALLBACK {
        match fetch_releases(ch).await {
            Ok(r) if !r.releases.is_empty() => return Ok(r),
            Ok(_) => last_err = format!("{ch}: empty"),
            Err(e) => last_err = format!("{ch}: {e}"),
        }
    }
    Err(format!("No releases available on any channel ({last_err})"))
}

// ---------------------------------------------------------------------------
// Command: list_block_devices
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_block_devices() -> Result<Vec<BlockDevice>, String> {
    #[cfg(target_os = "macos")]
    {
        list_block_devices_macos().await
    }
    #[cfg(target_os = "linux")]
    {
        list_block_devices_linux().await
    }
    #[cfg(target_os = "windows")]
    {
        list_block_devices_windows().await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err("Disk detection not supported on this platform".to_string())
    }
}

#[cfg(target_os = "windows")]
async fn list_block_devices_windows() -> Result<Vec<BlockDevice>, String> {
    // Enumeration touches the kernel; run it off the async runtime.
    let disks = tokio::task::spawn_blocking(flasher_core::win::list_disks)
        .await
        .map_err(|e| format!("Disk enumeration task failed: {e}"))??;

    let mut devices = Vec::new();
    for d in disks {
        // Same guard as macOS/Linux: only removable, non-system, <=2TB targets.
        if !d.removable || d.is_system || d.size == 0 || d.size > 2_000_000_000_000 {
            continue;
        }
        let name = if d.bus.is_empty() || d.bus == "Other" {
            d.model.clone()
        } else {
            format!("{} ({})", d.model, d.bus)
        };
        devices.push(BlockDevice {
            path: d.id,
            name,
            size: d.size,
            size_human: format_bytes(d.size),
            removable: true,
            // Windows auto-mounts volumes; we don't surface mount points here.
            mounted: false,
            mount_points: vec![],
        });
    }
    Ok(devices)
}

#[cfg(target_os = "macos")]
async fn list_block_devices_macos() -> Result<Vec<BlockDevice>, String> {
    let output = tokio::process::Command::new("diskutil")
        .args(["list", "-plist", "physical"])
        .output()
        .await
        .map_err(|e| format!("Failed to run diskutil list: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "diskutil list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let plist_value: plist::Value = plist::from_bytes(&output.stdout)
        .map_err(|e| format!("Failed to parse diskutil plist: {e}"))?;
    let disk_ids = plist_value
        .as_dictionary()
        .and_then(|d| d.get("AllDisksAndPartitions"))
        .and_then(|v| v.as_array())
        .ok_or("Unexpected diskutil output format")?;

    let mut devices = Vec::new();
    for disk_entry in disk_ids {
        let dict = match disk_entry.as_dictionary() {
            Some(d) => d,
            None => continue,
        };
        let device_id = match dict.get("DeviceIdentifier").and_then(|v| v.as_string()) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let path = format!("/dev/{device_id}");

        let info_output = tokio::process::Command::new("diskutil")
            .args(["info", "-plist", &path])
            .output()
            .await
            .map_err(|e| format!("Failed to run diskutil info: {e}"))?;
        if !info_output.status.success() {
            continue;
        }
        let info_plist: plist::Value = match plist::from_bytes(&info_output.stdout) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let info = match info_plist.as_dictionary() {
            Some(d) => d,
            None => continue,
        };

        let size = match info.get("TotalSize").and_then(|v| v.as_unsigned_integer()) {
            Some(s) if s > 0 && s <= 2_000_000_000_000 => s,
            _ => continue, // unknown, zero, or >2TB (likely an external HDD/SSD)
        };
        let name = info
            .get("MediaName")
            .and_then(|v| v.as_string())
            .unwrap_or("Unknown Device")
            .to_string();
        // The removable flag — NOT `Internal` — is the system-disk guard: the
        // built-in SD reader reports `Internal = true` but `Ejectable = true`,
        // while the boot disk is `Ejectable = false`. Filtering on `Internal`
        // would wrongly hide built-in card readers.
        let removable = info
            .get("Ejectable")
            .and_then(|v| v.as_boolean())
            .or_else(|| {
                info.get("RemovableMediaOrExternalDevice")
                    .and_then(|v| v.as_boolean())
            })
            .unwrap_or(false);
        if !removable {
            continue;
        }

        let mut mount_points: Vec<String> = Vec::new();
        if let Some(mp) = info.get("MountPoint").and_then(|v| v.as_string()) {
            if !mp.is_empty() {
                mount_points.push(mp.to_string());
            }
        }
        if let Some(parts) = dict.get("Partitions").and_then(|v| v.as_array()) {
            for part in parts {
                if let Some(mp) = part
                    .as_dictionary()
                    .and_then(|p| p.get("MountPoint"))
                    .and_then(|v| v.as_string())
                {
                    if !mp.is_empty() {
                        mount_points.push(mp.to_string());
                    }
                }
            }
        }
        let mounted = !mount_points.is_empty();

        devices.push(BlockDevice {
            path,
            name,
            size,
            size_human: format_bytes(size),
            removable,
            mounted,
            mount_points,
        });
    }
    Ok(devices)
}

#[cfg(target_os = "linux")]
async fn list_block_devices_linux() -> Result<Vec<BlockDevice>, String> {
    let output = tokio::process::Command::new("lsblk")
        .args([
            "--json",
            "--output",
            "NAME,SIZE,RM,TYPE,MOUNTPOINT,MODEL",
            "--bytes",
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run lsblk: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse lsblk output: {e}"))?;
    let blockdevices = json
        .get("blockdevices")
        .and_then(|v| v.as_array())
        .ok_or("Unexpected lsblk output format")?;

    let mut devices = Vec::new();
    for dev in blockdevices {
        let dev_type = dev.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let removable = dev.get("rm").and_then(|v| v.as_bool()).unwrap_or(false);
        if dev_type != "disk" || !removable {
            continue;
        }
        let name_str = dev.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let path = format!("/dev/{name_str}");
        let size = dev
            .get("size")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        if size == 0 || size > 2_000_000_000_000 {
            continue;
        }
        let model = dev
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Device")
            .trim()
            .to_string();
        let mountpoint = dev.get("mountpoint").and_then(|v| v.as_str()).unwrap_or("");
        let mounted = !mountpoint.is_empty();
        let mount_points = if mounted {
            vec![mountpoint.to_string()]
        } else {
            vec![]
        };

        devices.push(BlockDevice {
            path,
            name: model,
            size,
            size_human: format_bytes(size),
            removable: true,
            mounted,
            mount_points,
        });
    }
    Ok(devices)
}

// ---------------------------------------------------------------------------
// Command: download_image
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn download_image(
    app_handle: AppHandle,
    state: tauri::State<'_, FlasherState>,
    download_url: String,
    checksum_url: String,
) -> Result<DownloadResult, String> {
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();

    let checksum_raw = reqwest::get(&checksum_url)
        .await
        .map_err(|e| format!("Failed to fetch checksum: {e}"))?
        .text()
        .await
        .map_err(|e| format!("Failed to read checksum: {e}"))?;
    let raw_hash = checksum_raw.split_whitespace().next().unwrap_or("");
    let checksum = parse_sha256(raw_hash)?;

    let cache_dir = app_handle
        .path()
        .app_cache_dir()
        .map_err(|e| format!("Failed to resolve app cache dir: {e}"))?
        .join("images");
    std::fs::create_dir_all(&cache_dir).map_err(|e| format!("Failed to create cache dir: {e}"))?;

    let filename = filename_from_url(&download_url);
    let image_path = cache_dir.join(&filename);

    if image_path.exists() {
        // Fast path: a marker written after a prior verified download lets us
        // reuse the cached image instantly, without re-hashing gigabytes on
        // every run (that re-hash is what made cached flashes look like a
        // fresh download).
        if marker_validates(&image_path, &checksum) {
            return Ok(DownloadResult {
                image_path: image_path.to_string_lossy().to_string(),
                checksum,
                cached: true,
            });
        }
        // Older cache without a marker (or a size mismatch): verify once, then
        // record a marker so subsequent runs take the fast path.
        if verify_file_checksum(&image_path, &checksum)? {
            if let Ok(meta) = std::fs::metadata(&image_path) {
                write_marker(&image_path, &checksum, meta.len());
            }
            return Ok(DownloadResult {
                image_path: image_path.to_string_lossy().to_string(),
                checksum,
                cached: true,
            });
        }
        let _ = std::fs::remove_file(&image_path);
        let _ = std::fs::remove_file(marker_path(&image_path));
    }

    let response = reqwest::get(&download_url)
        .await
        .map_err(|e| format!("Failed to start download: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("Download failed: HTTP {}", response.status()));
    }
    let total_bytes = response.content_length().unwrap_or(0);

    let mut file = std::fs::File::create(&image_path)
        .map_err(|e| format!("Failed to create cache file: {e}"))?;
    let mut downloaded_bytes: u64 = 0;
    let mut hasher = Sha256::new();
    let mut stream = response.bytes_stream();
    let start_time = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();

    while let Some(chunk_result) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&image_path);
            return Err("Download cancelled".to_string());
        }
        let chunk = chunk_result.map_err(|e| format!("Download error: {e}"))?;
        file.write_all(&chunk)
            .map_err(|e| format!("Failed to write chunk: {e}"))?;
        hasher.update(&chunk);
        downloaded_bytes += chunk.len() as u64;

        if last_emit.elapsed().as_millis() >= PROGRESS_THROTTLE_MS {
            let elapsed = start_time.elapsed().as_secs_f64();
            let speed_bps = if elapsed > 0.0 {
                (downloaded_bytes as f64 / elapsed) as u64
            } else {
                0
            };
            let percent = if total_bytes > 0 {
                (downloaded_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            let _ = app_handle.emit(
                "flasher:download-progress",
                DownloadProgress {
                    downloaded_bytes,
                    total_bytes,
                    percent,
                    speed_bps,
                },
            );
            last_emit = std::time::Instant::now();
        }
    }

    let _ = app_handle.emit(
        "flasher:download-progress",
        DownloadProgress {
            downloaded_bytes,
            total_bytes,
            percent: 100.0,
            speed_bps: 0,
        },
    );

    // Verify what we wrote against the published checksum (the hash was
    // computed incrementally, so this costs nothing extra), then drop a marker
    // so the next flash of this image reuses the cache without re-downloading
    // or re-hashing.
    let actual = format!("{:x}", hasher.finalize());
    if actual != checksum.to_ascii_lowercase() {
        let _ = std::fs::remove_file(&image_path);
        return Err(format!(
            "Checksum mismatch after download (expected {checksum}, got {actual})"
        ));
    }
    write_marker(&image_path, &checksum, downloaded_bytes);

    Ok(DownloadResult {
        image_path: image_path.to_string_lossy().to_string(),
        checksum,
        cached: false,
    })
}

fn verify_file_checksum(path: &std::path::Path, expected: &str) -> Result<bool, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Failed to open file for checksum: {e}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("Failed to hash file: {e}"))?;
    let hash = format!("{:x}", hasher.finalize());
    Ok(hash == expected.to_ascii_lowercase())
}

fn marker_path(image_path: &std::path::Path) -> std::path::PathBuf {
    let mut name = image_path.as_os_str().to_owned();
    name.push(".cache");
    std::path::PathBuf::from(name)
}

fn read_marker(image_path: &std::path::Path) -> Option<(String, u64)> {
    let content = std::fs::read_to_string(marker_path(image_path)).ok()?;
    let mut parts = content.split_whitespace();
    let sha = parts.next()?.to_string();
    let size = parts.next()?.parse::<u64>().ok()?;
    Some((sha, size))
}

fn write_marker(image_path: &std::path::Path, checksum: &str, size: u64) {
    let _ = std::fs::write(marker_path(image_path), format!("{checksum} {size}"));
}

/// A cached image is trusted without re-hashing when a marker from a prior
/// verified download records the same checksum and the file is still that size.
fn marker_validates(image_path: &std::path::Path, expected: &str) -> bool {
    match (read_marker(image_path), std::fs::metadata(image_path)) {
        (Some((sha, size)), Ok(meta)) => sha.eq_ignore_ascii_case(expected) && meta.len() == size,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Command: flash_image (decompress if needed, then dd)
// ---------------------------------------------------------------------------

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn flash_image(
    app_handle: AppHandle,
    state: tauri::State<'_, FlasherState>,
    image_path: String,
    target_disk: String,
    wifi: Option<WifiConfig>,
    hostname: Option<String>,
    init_format: flasher_core::InitFormat,
) -> Result<(), String> {
    validate_disk_path(&target_disk)?;
    let src_path = validate_image_file(std::path::Path::new(&image_path))?;

    // Re-validate the target is still a removable external device.
    let devices = list_block_devices().await?;
    if !devices.iter().any(|d| d.path == target_disk) {
        return Err(format!(
            "Target disk {target_disk} is not a valid removable device"
        ));
    }

    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();

    let temp_dir = std::env::temp_dir().join("aircast-flasher");
    std::fs::create_dir_all(&temp_dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;

    // Decompress only if compressed; raw .img is written directly (local files).
    let lower = src_path.to_string_lossy().to_ascii_lowercase();
    let (write_path, is_temp) = if lower.ends_with(".xz") || lower.ends_with(".gz") {
        check_available_space(&temp_dir, MIN_TEMP_SPACE_BYTES)?;
        let dst = temp_dir.join("aircast-image.img");
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
        }
        decompress(&app_handle, &src_path, &dst, &lower, cancel.clone()).await?;
        (dst, true)
    } else {
        (src_path.clone(), false)
    };

    // Customization runs inside the engine, so fold the provisioning config into
    // the single elevated flash. base64(serde_json(ProvisionConfig)) → helper.
    let config = ProvisionConfig {
        hostname,
        wifi,
        init_format,
    };
    let provision_b64 = {
        use base64::Engine as _;
        let json = serde_json::to_vec(&config)
            .map_err(|e| format!("Failed to encode provision config: {e}"))?;
        base64::engine::general_purpose::STANDARD.encode(json)
    };

    let total = std::fs::metadata(&write_path).map(|m| m.len()).unwrap_or(0);
    emit_flash_progress(&app_handle, FlashPhase::Writing, 0, total, 0.0);

    let write_result =
        run_elevated_flash(&app_handle, &target_disk, &write_path, &provision_b64).await;

    if is_temp {
        let _ = std::fs::remove_file(&write_path);
    }
    write_result
}

/// Map a helper progress `stage` string to a [`FlashPhase`].
fn phase_from_stage(stage: &str) -> Option<FlashPhase> {
    match stage {
        "write" => Some(FlashPhase::Writing),
        "verify" => Some(FlashPhase::Verifying),
        "customize" => Some(FlashPhase::Customizing),
        _ => None,
    }
}

/// Launch the elevated helper (this same binary `--flash-helper`) as
/// root/admin, while a tailer thread reads the progress file it appends to and
/// re-emits `flasher:flash-progress`. The elevation mechanism is per-OS.
async fn run_elevated_flash(
    app_handle: &AppHandle,
    target_disk: &str,
    image: &std::path::Path,
    provision_b64: &str,
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Failed to find current exe: {e}"))?;

    // A unique progress file in the temp dir, shared with the elevated helper.
    let progress_file = std::env::temp_dir().join(format!(
        "aircast-flash-progress-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&progress_file);
    std::fs::File::create(&progress_file)
        .map_err(|e| format!("Failed to create progress file: {e}"))?;

    // Tailer: poll the progress file and emit flash progress until told to stop.
    let app = app_handle.clone();
    let pf_for_tail = progress_file.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_tail = stop.clone();
    let tail = tokio::spawn(async move {
        let mut offset: u64 = 0;
        loop {
            if let Ok(bytes) = std::fs::read(&pf_for_tail) {
                if (bytes.len() as u64) > offset {
                    let new = String::from_utf8_lossy(&bytes[offset as usize..]).to_string();
                    offset = bytes.len() as u64;
                    for line in new.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                            let stage = v.get("stage").and_then(|x| x.as_str());
                            let phase = stage.and_then(phase_from_stage);
                            if let Some(phase) = phase {
                                let done = v.get("done").and_then(|x| x.as_u64()).unwrap_or(0);
                                let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0);
                                let percent = if total > 0 {
                                    (done as f64 / total as f64) * 100.0
                                } else {
                                    0.0
                                };
                                emit_flash_progress(&app, phase, done, total, percent);
                            }
                        }
                    }
                }
            }
            if stop_tail.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(
                PROGRESS_THROTTLE_MS as u64,
            ))
            .await;
        }
    });

    let launch = launch_helper(&exe, target_disk, image, &progress_file, provision_b64).await;

    // Stop the tailer regardless of outcome.
    stop.store(true, Ordering::SeqCst);
    let _ = tail.await;

    // Re-read the progress file for a terminal error line.
    let contents = std::fs::read_to_string(&progress_file).unwrap_or_default();
    let last_error = contents
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .find_map(|v| {
            v.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        });
    let _ = std::fs::remove_file(&progress_file);

    match launch {
        Ok(true) => Ok(()),
        Ok(false) => Err(last_error.unwrap_or_else(|| {
            "Elevated flash failed (authorization declined or write error)".to_string()
        })),
        Err(e) => Err(last_error.unwrap_or(e)),
    }
}

/// Per-OS elevation launcher. Returns `Ok(true)` if the helper exited 0,
/// `Ok(false)` on a non-zero exit, `Err` if the launcher itself failed.
#[cfg(target_os = "macos")]
async fn launch_helper(
    exe: &std::path::Path,
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_b64: &str,
) -> Result<bool, String> {
    use crate::macos_auth::Authorization;

    let exe = exe.to_string_lossy().to_string();
    let device = target_disk.to_string();
    let image = image.to_string_lossy().to_string();
    let pf = progress_file.to_string_lossy().to_string();
    let provision = provision_b64.to_string();

    // One Touch ID prompt; the helper inherits root and holds the device open.
    tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let auth = Authorization::new()?;
        match auth.execute(
            &exe,
            &[
                "--flash-helper",
                "--device",
                &device,
                "--image",
                &image,
                "--progress-file",
                &pf,
                "--provision",
                &provision,
            ],
        ) {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.contains("Authentication cancelled") {
                    Err(e)
                } else {
                    // Non-zero exit from the helper → surface the progress error.
                    Ok(false)
                }
            }
        }
    })
    .await
    .map_err(|e| format!("Elevated flash task failed: {e}"))?
}

#[cfg(target_os = "linux")]
async fn launch_helper(
    exe: &std::path::Path,
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_b64: &str,
) -> Result<bool, String> {
    let status = tokio::process::Command::new("pkexec")
        .arg(exe)
        .args([
            "--flash-helper",
            "--device",
            target_disk,
            "--image",
            &image.to_string_lossy(),
            "--progress-file",
            &progress_file.to_string_lossy(),
            "--provision",
            provision_b64,
        ])
        .status()
        .await
        .map_err(|e| format!("Failed to launch elevated flasher via pkexec: {e}"))?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
fn hidden_command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

#[cfg(target_os = "windows")]
async fn launch_helper(
    exe: &std::path::Path,
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_b64: &str,
) -> Result<bool, String> {
    // Windows has no pkexec/Authorization-Services: re-exec this binary elevated
    // via PowerShell's Start-Process -Verb RunAs (UAC) and wait for it.
    let ps_cmd = format!(
        "$p = Start-Process -FilePath '{exe}' -ArgumentList \
         '--flash-helper','--device','{device}','--image','{image}','--progress-file','{pf}','--provision','{provision}' \
         -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        exe = ps_escape(&exe.to_string_lossy()),
        device = ps_escape(target_disk),
        image = ps_escape(&image.to_string_lossy()),
        pf = ps_escape(&progress_file.to_string_lossy()),
        provision = ps_escape(provision_b64),
    );

    let status = hidden_command("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .status()
        .await
        .map_err(|e| format!("Failed to launch elevated flasher: {e}"))?;
    Ok(status.success())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn launch_helper(
    _exe: &std::path::Path,
    _target_disk: &str,
    _image: &std::path::Path,
    _progress_file: &std::path::Path,
    _provision_b64: &str,
) -> Result<bool, String> {
    Err("Disk flashing is not supported on this platform".to_string())
}

/// Escape a string for embedding inside a single-quoted PowerShell literal
/// (the only special char inside `'...'` is a single quote, doubled).
#[cfg(target_os = "windows")]
fn ps_escape(s: &str) -> String {
    s.replace('\'', "''")
}

async fn decompress(
    app_handle: &AppHandle,
    src: &std::path::Path,
    dst: &std::path::Path,
    lower_name: &str,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    emit_flash_progress(app_handle, FlashPhase::Decompressing, 0, 0, 0.0);

    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    let app_handle = app_handle.clone();
    let is_gz = lower_name.ends_with(".gz");

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&src).map_err(|e| format!("Failed to open image: {e}"))?;
        let mut decoder: Box<dyn Read> = if is_gz {
            Box::new(flate2::read::GzDecoder::new(file))
        } else {
            Box::new(xz2::read::XzDecoder::new(file))
        };
        let mut output = std::fs::File::create(&dst)
            .map_err(|e| format!("Failed to create decompressed file: {e}"))?;

        let mut buffer = vec![0u8; 1024 * 1024];
        let mut bytes_written: u64 = 0;
        let mut last_emit = std::time::Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dst);
                return Err("Flash cancelled".to_string());
            }
            let n = decoder
                .read(&mut buffer)
                .map_err(|e| format!("Decompression error: {e}"))?;
            if n == 0 {
                break;
            }
            output
                .write_all(&buffer[..n])
                .map_err(|e| format!("Write error: {e}"))?;
            bytes_written += n as u64;
            if last_emit.elapsed().as_millis() >= PROGRESS_THROTTLE_MS {
                // total=0 → UI shows an indeterminate "decompressing N MB" bar.
                emit_flash_progress(
                    &app_handle,
                    FlashPhase::Decompressing,
                    bytes_written,
                    0,
                    0.0,
                );
                last_emit = std::time::Instant::now();
            }
        }
        emit_flash_progress(
            &app_handle,
            FlashPhase::Decompressing,
            bytes_written,
            bytes_written,
            100.0,
        );
        Ok(bytes_written)
    })
    .await
    .map_err(|e| format!("Decompress task failed: {e}"))?
}

// ---------------------------------------------------------------------------
// Command: cancel_flash
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn cancel_flash(state: tauri::State<'_, FlasherState>) -> Result<(), String> {
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

// ---------------------------------------------------------------------------
// Command: list_wifi_networks (so the user can pick instead of typing)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct WifiNetworks {
    /// The currently-joined SSID, if any (used to prefill the form).
    pub current: Option<String>,
    /// Selectable SSIDs: known/preferred networks on macOS (no Location
    /// permission needed); a live scan on Linux.
    pub known: Vec<String>,
    /// Detected 2-letter WiFi regulatory domain from the system locale, so the
    /// user doesn't have to type it. `None` if it can't be determined.
    pub country: Option<String>,
}

/// Extract the 2-letter region from a locale like `en_US`, `ka_GE`,
/// `en_US@rg=...`, or `zh-Hans-CN`. Returns uppercase, or None.
fn region_from_locale(locale: &str) -> Option<String> {
    let base = locale.split('@').next().unwrap_or(locale);
    let parts: Vec<&str> = base.split(['_', '-']).filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return None; // language only, no region
    }
    let region = parts.last()?;
    if region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(region.to_uppercase())
    } else {
        None
    }
}

#[tauri::command]
pub async fn list_wifi_networks() -> Result<WifiNetworks, String> {
    #[cfg(target_os = "macos")]
    {
        list_wifi_macos().await
    }
    #[cfg(target_os = "linux")]
    {
        list_wifi_linux().await
    }
    #[cfg(target_os = "windows")]
    {
        list_wifi_windows().await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Ok(WifiNetworks {
            current: None,
            known: vec![],
            country: None,
        })
    }
}

#[cfg(target_os = "windows")]
async fn list_wifi_windows() -> Result<WifiNetworks, String> {
    // Known/saved networks: `netsh wlan show profiles` lists
    //   "    All User Profile     : <NAME>"
    let known = hidden_command("netsh")
        .args(["wlan", "show", "profiles"])
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.contains("All User Profile"))
                })
                .map(|(_, v)| v.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Current SSID: `netsh wlan show interfaces` has a line
    //   "    SSID                   : <NAME>"
    // Careful: there is also a "BSSID" line — match the SSID line specifically.
    let current = hidden_command("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
        .await
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| {
                    let k = k.trim();
                    k == "SSID"
                })
                .map(|(_, v)| v.trim().to_string())
        })
        .filter(|s| !s.is_empty());

    // Region from the user locale, e.g. "en-US" → "US".
    let country = user_locale_windows()
        .await
        .and_then(|l| region_from_locale(&l));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

/// Best-effort user locale name on Windows (e.g. "en-US"), via PowerShell's
/// Get-Culture, falling back to the LANG/locale env if PowerShell is missing.
#[cfg(target_os = "windows")]
async fn user_locale_windows() -> Option<String> {
    if let Ok(o) = hidden_command("powershell")
        .args(["-NoProfile", "-Command", "(Get-Culture).Name"])
        .output()
        .await
    {
        let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .ok()
        .map(|l| l.split('.').next().unwrap_or(&l).to_string())
}

#[cfg(target_os = "macos")]
async fn wifi_device_macos() -> String {
    // Find the Wi-Fi hardware port's device (usually en0), fall back to en0.
    if let Ok(out) = tokio::process::Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut wifi = false;
        for line in text.lines() {
            if line.contains("Wi-Fi") || line.contains("AirPort") {
                wifi = true;
            } else if wifi {
                if let Some(dev) = line.strip_prefix("Device: ") {
                    return dev.trim().to_string();
                }
            }
        }
    }
    "en0".to_string()
}

#[cfg(target_os = "macos")]
async fn list_wifi_macos() -> Result<WifiNetworks, String> {
    let dev = wifi_device_macos().await;

    // Currently-joined SSID (may be "You are not associated…").
    let current = tokio::process::Command::new("networksetup")
        .args(["-getairportnetwork", &dev])
        .output()
        .await
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_once("Current Wi-Fi Network: ")
                .map(|(_, name)| name.trim().to_string())
        })
        .filter(|s| !s.is_empty());

    // Preferred/known networks — no Location permission required.
    let known = tokio::process::Command::new("networksetup")
        .args(["-listpreferredwirelessnetworks", &dev])
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1) // "Preferred networks on enN:"
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Region from the system locale (e.g. AppleLocale "ka_GE" → "GE").
    let country = tokio::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .await
        .ok()
        .and_then(|o| region_from_locale(String::from_utf8_lossy(&o.stdout).trim()));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

#[cfg(target_os = "linux")]
async fn list_wifi_linux() -> Result<WifiNetworks, String> {
    // Live scan via NetworkManager. `-t` terse, `-f` fields.
    let out = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "ACTIVE,SSID", "dev", "wifi"])
        .output()
        .await
        .map_err(|e| format!("Failed to run nmcli: {e}"))?;

    let mut current = None;
    let mut known = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // Format: "yes:MySSID" / "no:OtherSSID"
        let (active, ssid) = line.split_once(':').unwrap_or(("no", line));
        let ssid = ssid.trim();
        if ssid.is_empty() {
            continue;
        }
        if active == "yes" {
            current = Some(ssid.to_string());
        }
        if !known.iter().any(|s| s == ssid) {
            known.push(ssid.to_string());
        }
    }

    // Region from the locale env (e.g. LANG "en_US.UTF-8" → "US").
    let country = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .ok()
        .and_then(|l| region_from_locale(l.split('.').next().unwrap_or(&l)));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn emit_flash_progress(
    app_handle: &AppHandle,
    phase: FlashPhase,
    bytes_processed: u64,
    total_bytes: u64,
    percent: f64,
) {
    let _ = app_handle.emit(
        "flasher:flash-progress",
        FlashProgress {
            phase,
            bytes_processed,
            total_bytes,
            percent,
        },
    );
}

fn format_bytes(bytes: u64) -> String {
    let unit = byte_unit::Byte::from_u64(bytes);
    let adjusted = unit.get_appropriate_unit(byte_unit::UnitType::Decimal);
    format!("{adjusted:.1}")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_disk_path_rejects_injection() {
        assert!(validate_disk_path("/dev/disk2; rm -rf /").is_err());
        assert!(validate_disk_path("/dev/disk2$(whoami)").is_err());
        assert!(validate_disk_path("../../etc/passwd").is_err());
        assert!(validate_disk_path("").is_err());
    }

    #[test]
    fn validate_disk_path_rejects_partitions() {
        if cfg!(target_os = "macos") {
            assert!(validate_disk_path("/dev/disk2s1").is_err());
            assert!(validate_disk_path("/dev/disk2").is_ok());
        }
    }

    #[test]
    fn parse_sha256_normalizes_and_rejects() {
        assert_eq!(parse_sha256(&"A".repeat(64)), Ok("a".repeat(64)));
        assert!(parse_sha256("abc").is_err());
        assert!(parse_sha256("<html>nope</html>").is_err());
    }

    #[test]
    fn marker_validates_trusts_matching_checksum_and_size() {
        let dir = std::env::temp_dir().join(format!("aircast-cache-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("image.img.xz");
        std::fs::write(&img, b"hello world").unwrap();
        let checksum = "a".repeat(64);

        // No marker yet -> not trusted (forces a full verify / re-download).
        assert!(!marker_validates(&img, &checksum));

        write_marker(&img, &checksum, 11);
        assert!(marker_validates(&img, &checksum));
        // Checksum comparison is case-insensitive.
        assert!(marker_validates(&img, &checksum.to_ascii_uppercase()));
        // A different published checksum invalidates the cache.
        assert!(!marker_validates(&img, &"b".repeat(64)));

        // A size change (truncation/corruption) invalidates the cache.
        std::fs::write(&img, b"hello").unwrap();
        assert!(!marker_validates(&img, &checksum));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn filename_from_url_strips_query() {
        assert_eq!(
            filename_from_url("https://downloads.aircast.one/images/v1/aircast.img.xz?t=1"),
            "aircast.img.xz"
        );
        assert_eq!(filename_from_url("not-a-url"), "image.img.xz");
    }

    #[test]
    fn flash_phase_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FlashPhase::Writing).unwrap(),
            "\"writing\""
        );
        assert_eq!(
            serde_json::to_string(&FlashPhase::Decompressing).unwrap(),
            "\"decompressing\""
        );
    }

    #[test]
    fn region_from_locale_parses() {
        assert_eq!(region_from_locale("en_US"), Some("US".into()));
        assert_eq!(region_from_locale("ka_GE"), Some("GE".into()));
        assert_eq!(region_from_locale("en_US@rg=uszzzz"), Some("US".into()));
        assert_eq!(region_from_locale("zh-Hans-CN"), Some("CN".into()));
        assert_eq!(region_from_locale("en"), None);
        assert_eq!(region_from_locale("C"), None);
    }

    #[test]
    fn releases_response_deserializes() {
        let json = r#"{"releases":[{"version":"v1.0.0","prerelease":false,"created_at":"2026-06-01T00:00:00Z","image":{"filename":"a.img.xz","extension":"img.xz","size":100,"download_url":"https://x/a.img.xz","checksum_url":"https://x/a.img.xz.sha256"}}]}"#;
        let r: ReleasesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.releases[0].version, "v1.0.0");
    }
}
