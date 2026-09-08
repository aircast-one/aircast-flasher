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
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};

use flasher_core::{ProvisionConfig, TailscaleConfig, WifiConfig};

use crate::telemetry;

const PROGRESS_THROTTLE_MS: u128 = 100;
const MIN_TEMP_SPACE_BYTES: u64 = 4_500_000_000; // ~4.5 GB

/// Smallest card that can hold a decompressed aircast-lite image, used to warn
/// before the download when the release does not publish its uncompressed size.
/// v0.3.2 decompresses to 2.97 GB (`xz --robot -l`), so this rejects 2 GB cards
/// and clears every real 4 GB one. It is a floor, not the true requirement:
/// the exact check runs against the decompressed file in `flash_image_inner`.
const MIN_CARD_BYTES: u64 = 3_500_000_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
const SPEED_WINDOW: Duration = Duration::from_secs(1);
const BACKOFF_BASE: Duration = Duration::from_secs(1);
const BACKOFF_CAP: Duration = Duration::from_secs(30);
const CANCEL_POLL: Duration = Duration::from_millis(100);

/// How many *consecutive* attempts may move zero bytes before the download is
/// declared dead. Any progress resets the count, so a long transfer over a link
/// that drops repeatedly still finishes; only a link that stops delivering
/// gives up.
const STALLED_ATTEMPTS: usize = 6;

/// Absolute backstop, since [`STALLED_ATTEMPTS`] resets on any progress at all:
/// a server dribbling one byte per connection would otherwise retry forever.
const MAX_RETRIES: usize = 100;

/// How long to wait for the elevated helper to notice an aborted preparation
/// before reporting the preparation's own error. Long enough for a helper that
/// is actually running to read the abort file, short enough that an unanswered
/// elevation prompt does not hold the UI.
const HELPER_ABORT_GRACE: Duration = Duration::from_secs(5);

fn http() -> &'static reqwest::Client {
    static CLIENT: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .read_timeout(STALL_TIMEOUT)
            .build()
            .expect("http client: TLS backend failed to initialise")
    })
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// Shared cancel flag for the active download/flash. `cancel_flash` flips it;
/// long-running loops poll it.
/// `cancel` stops in-flight work (download loops, decompress) and is also set
/// internally when the elevated helper dies first; `user_cancel` records that
/// the user pressed Cancel, so telemetry can tell a cancellation from a failure.
pub struct FlasherState {
    pub cancel: Arc<AtomicBool>,
    pub user_cancel: Arc<AtomicBool>,
}

impl FlasherState {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            user_cancel: Arc::new(AtomicBool::new(false)),
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
    /// Size of the *download* — the compressed `.img.xz`. The card has to hold
    /// the decompressed image, which is several times larger, so this must
    /// never be compared against a card's capacity.
    pub size: u64,
    /// Size of the decompressed image, when the release publishes it. Absent
    /// today, so the UI falls back to [`MIN_CARD_BYTES`].
    #[serde(default)]
    pub uncompressed_size: Option<u64>,
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
    /// Stamped from [`MIN_CARD_BYTES`] on the way out, so the card-size warning
    /// has one source of truth instead of a constant copied into the frontend.
    #[serde(default)]
    pub min_card_bytes: u64,
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
    /// Rate of the phase in flight. The write is the longest wait in the app,
    /// so a bar alone answers "is it moving" but not "how long".
    pub speed_bps: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
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
fn private_temp_dir() -> Result<std::path::PathBuf, String> {
    let dir = std::env::temp_dir().join("aircast-flasher");
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create temp dir: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        let meta = std::fs::symlink_metadata(&dir)
            .map_err(|e| format!("Failed to stat temp dir: {e}"))?;
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
fn fits_on_card(image_bytes: u64, card_size: u64, card_label: &str) -> Result<(), String> {
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
        "development" => Ok("downloads-dev.aircast.one"),
        "staging" => Ok("downloads.stage.aircast.one"),
        other => Err(format!(
            "Unknown channel: {other} (expected stable|development|staging)"
        )),
    }
}

async fn fetch_releases(channel: &str) -> Result<ReleasesResponse, String> {
    let host = channel_host(channel)?;
    let url = format!("https://{host}/lite/releases.json");
    let response = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch releases: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<ReleasesResponse>()
        .await
        .map(|r| ReleasesResponse {
            min_card_bytes: MIN_CARD_BYTES,
            ..r
        })
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
    job_id: String,
    download_url: String,
    checksum_url: String,
) -> Result<DownloadResult, String> {
    let started = std::time::Instant::now();
    let cancelled = state.user_cancel.clone();
    let mut stats = DownloadStats::default();
    let result = download_image_inner(
        &app_handle,
        &state,
        download_url,
        checksum_url,
        &mut stats,
    )
    .await;
    let duration_ms = started.elapsed().as_millis() as u64;

    let bytes = result
        .as_ref()
        .ok()
        .and_then(|r| std::fs::metadata(&r.image_path).ok().map(|m| m.len()))
        .unwrap_or(0);
    let cached = result.as_ref().map(|r| r.cached).unwrap_or(false);
    let outcome = match &result {
        Ok(_) => "success",
        Err(_) if cancelled.load(Ordering::SeqCst) => "cancelled",
        Err(_) => "failed",
    };

    telemetry::record(
        &app_handle,
        &telemetry::DownloadEvent {
            envelope: telemetry::Envelope::new(
                "download",
                job_id,
                &app_handle,
                outcome,
                duration_ms,
                result.as_ref().err().cloned(),
            ),
            bytes,
            cached,
            mbps: (!cached)
                .then(|| telemetry::mbps(bytes, stats.transfer_ms))
                .flatten(),
            retries: stats.retries,
            restarts: stats.restarts,
            resumed_bytes: stats.resumed_bytes,
        },
    );

    result
}

async fn download_image_inner(
    app_handle: &AppHandle,
    state: &tauri::State<'_, FlasherState>,
    download_url: String,
    checksum_url: String,
    stats: &mut DownloadStats,
) -> Result<DownloadResult, String> {
    state.user_cancel.store(false, Ordering::SeqCst);
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();

    let checksum_raw = http()
        .get(&checksum_url)
        .send()
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

    // A partial file from a download that died mid-flight: pick it up where it
    // stopped. Without this, a dropped connection the retry loop could not
    // outlast costs the whole transfer again on the next attempt.
    let resumed = resume_partial(&image_path, &checksum).await;

    if resumed.is_none() && image_path.exists() {
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
        let _ = std::fs::remove_file(part_path(&image_path));
    }

    let mut sink = match resumed {
        Some(sink) => sink,
        None => Sink {
            file: tokio::fs::File::create(&image_path)
                .await
                .map_err(|e| format!("Failed to create cache file: {e}"))?,
            hasher: Sha256::new(),
            written: 0,
            total: 0,
        },
    };

    let mut emit = |p: &DownloadProgress| {
        let _ = app_handle.emit("flasher:download-progress", p);
    };

    let outcome = {
        let mut stalled: usize = 0;
        loop {
            let attempt_started = std::time::Instant::now();
            let before = sink.written;
            let result = stream_into(&mut emit, &cancel, &download_url, &mut sink, stats).await;
            stats.transfer_ms += attempt_started.elapsed().as_millis() as u64;

            match result {
                Ok(()) => break Ok(()),
                Err(Fail::Fatal(msg)) => break Err(msg),
                Err(Fail::Transient(msg)) => {
                    stalled = if sink.written > before { 0 } else { stalled + 1 };
                    if stalled >= STALLED_ATTEMPTS || stats.retries >= MAX_RETRIES {
                        break Err(msg);
                    }
                    stats.retries += 1;
                    emit(&DownloadProgress {
                        downloaded_bytes: sink.written,
                        total_bytes: sink.total,
                        percent: percent_of(sink.written, sink.total),
                        speed_bps: 0,
                    });
                    if !sleep_unless_cancelled(&cancel, backoff(stalled)).await {
                        break Err("Download cancelled".to_string());
                    }
                }
            }
        }
    };

    if let Err(msg) = outcome {
        let _ = sink.file.flush().await;
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&image_path);
            let _ = std::fs::remove_file(part_path(&image_path));
        } else if sink.written > 0 {
            // Keep what arrived: the next attempt resumes from here instead of
            // starting the whole transfer again.
            let _ = std::fs::write(
                part_path(&image_path),
                format!("{checksum} {}", sink.written),
            );
        }
        return Err(msg);
    }

    sink.file
        .flush()
        .await
        .map_err(|e| format!("Failed to flush cache file: {e}"))?;

    let downloaded_bytes = sink.written;
    let total_bytes = sink.total;
    let hasher = sink.hasher;

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
        let _ = std::fs::remove_file(part_path(&image_path));
        return Err(format!(
            "Checksum mismatch after download (expected {checksum}, got {actual})"
        ));
    }
    let _ = std::fs::remove_file(part_path(&image_path));
    write_marker(&image_path, &checksum, downloaded_bytes);

    Ok(DownloadResult {
        image_path: image_path.to_string_lossy().to_string(),
        checksum,
        cached: false,
    })
}

struct Sink {
    file: tokio::fs::File,
    hasher: Sha256,
    written: u64,
    total: u64,
}

#[derive(Default)]
struct DownloadStats {
    retries: usize,
    restarts: usize,
    resumed_bytes: u64,
    transfer_ms: u64,
}

enum Fail {
    Transient(String),
    Fatal(String),
}

/// Reopen the leftovers of a download that died, so the next attempt continues
/// instead of refetching gigabytes. The recorded checksum binds the partial to
/// this exact release, and the file is truncated back to the recorded length so
/// a half-written final chunk cannot poison the resumed hash.
async fn resume_partial(image_path: &std::path::Path, checksum: &str) -> Option<Sink> {
    let (sha, bytes) = read_marker_file(&part_path(image_path))?;
    if bytes == 0 || !sha.eq_ignore_ascii_case(checksum) {
        return None;
    }
    if std::fs::metadata(image_path).ok()?.len() < bytes {
        return None;
    }

    let path = image_path.to_path_buf();
    let hasher = tokio::task::spawn_blocking(move || hash_prefix(&path, bytes))
        .await
        .ok()?
        .ok()?;

    let mut file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(image_path)
        .await
        .ok()?;
    file.set_len(bytes).await.ok()?;
    file.seek(std::io::SeekFrom::Start(bytes)).await.ok()?;

    Some(Sink {
        file,
        hasher,
        written: bytes,
        total: 0,
    })
}

/// SHA-256 of the first `bytes` bytes of `path`.
fn hash_prefix(path: &std::path::Path, bytes: u64) -> std::io::Result<Sha256> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1024 * 1024];
    let mut left = bytes;
    while left > 0 {
        let want = buf.len().min(left as usize);
        let n = file.read(&mut buf[..want])?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        left -= n as u64;
    }
    Ok(hasher)
}

fn percent_of(done: u64, total: u64) -> f64 {
    if total > 0 {
        ((done as f64 / total as f64) * 100.0).min(100.0)
    } else {
        0.0
    }
}

/// Exponential backoff, capped. No jitter: each install downloads on its own
/// schedule, so there is no herd to spread out.
fn backoff(stalled: usize) -> Duration {
    BACKOFF_CAP.min(BACKOFF_BASE * 2u32.saturating_pow(stalled.saturating_sub(1) as u32))
}

/// Sleep, giving up early if the flash is cancelled. Returns `false` if it was.
async fn sleep_unless_cancelled(cancel: &Arc<AtomicBool>, total: Duration) -> bool {
    let deadline = std::time::Instant::now() + total;
    while let Some(left) = deadline.checked_duration_since(std::time::Instant::now()) {
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        tokio::time::sleep(CANCEL_POLL.min(left)).await;
    }
    !cancel.load(Ordering::SeqCst)
}

/// Byte offset a `206 Partial Content` response starts at, per its
/// `Content-Range: bytes <start>-<end>/<len>` header.
fn content_range_start(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::CONTENT_RANGE)?
        .to_str()
        .ok()?
        .trim()
        .strip_prefix("bytes ")?
        .split('-')
        .next()?
        .trim()
        .parse()
        .ok()
}

/// One attempt at streaming the image into `sink`, resuming from
/// `sink.written` via a `Range` request. A dropped or stalled connection —
/// what a Wi-Fi switch or cellular handover looks like — is `Transient`, so
/// the caller retries and keeps the bytes already on disk.
async fn stream_into(
    progress: &mut (dyn FnMut(&DownloadProgress) + Send),
    cancel: &Arc<AtomicBool>,
    url: &str,
    sink: &mut Sink,
    stats: &mut DownloadStats,
) -> Result<(), Fail> {
    if cancel.load(Ordering::SeqCst) {
        return Err(Fail::Fatal("Download cancelled".to_string()));
    }

    let request = match sink.written {
        0 => http().get(url),
        n => http()
            .get(url)
            .header(reqwest::header::RANGE, format!("bytes={n}-")),
    };

    let response = request
        .send()
        .await
        .map_err(|e| Fail::Transient(format!("Download error: {e}")))?;

    if !response.status().is_success() {
        let status = response.status();
        let msg = format!("Download failed: HTTP {status}");
        return Err(if status.is_server_error() {
            Fail::Transient(msg)
        } else {
            Fail::Fatal(msg)
        });
    }

    // Where this response actually starts. A plain 200 restates the whole file;
    // a 206 is only usable if it begins exactly where the partial file ends —
    // a proxy answering 206 for a different range would otherwise be appended
    // at the wrong offset and only caught by the final checksum.
    let starts_at = if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
        content_range_start(response.headers())
    } else {
        Some(0)
    };

    if starts_at != Some(sink.written) {
        sink.file
            .set_len(0)
            .await
            .map_err(|e| Fail::Fatal(format!("Failed to restart download: {e}")))?;
        sink.file
            .seek(std::io::SeekFrom::Start(0))
            .await
            .map_err(|e| Fail::Fatal(format!("Failed to restart download: {e}")))?;
        sink.hasher = Sha256::new();
        sink.written = 0;
        stats.restarts += 1;
    } else if sink.written > 0 {
        stats.resumed_bytes += sink.written;
    }

    if sink.total == 0 {
        sink.total = response.content_length().unwrap_or(0) + sink.written;
    }

    let mut stream = response.bytes_stream();
    let mut last_emit = std::time::Instant::now();
    let mut window_start = std::time::Instant::now();
    let mut window_bytes: u64 = 0;
    let mut speed_bps: u64 = 0;

    while let Some(chunk_result) = stream.next().await {
        if cancel.load(Ordering::SeqCst) {
            return Err(Fail::Fatal("Download cancelled".to_string()));
        }
        let chunk = chunk_result.map_err(|e| Fail::Transient(format!("Download error: {e}")))?;
        sink.file
            .write_all(&chunk)
            .await
            .map_err(|e| Fail::Fatal(format!("Failed to write chunk: {e}")))?;
        sink.hasher.update(&chunk);
        sink.written += chunk.len() as u64;
        window_bytes += chunk.len() as u64;

        if window_start.elapsed() >= SPEED_WINDOW {
            speed_bps = (window_bytes as f64 / window_start.elapsed().as_secs_f64()) as u64;
            window_start = std::time::Instant::now();
            window_bytes = 0;
        }

        if last_emit.elapsed().as_millis() >= PROGRESS_THROTTLE_MS {
            progress(&DownloadProgress {
                downloaded_bytes: sink.written,
                total_bytes: sink.total,
                percent: percent_of(sink.written, sink.total),
                speed_bps,
            });
            last_emit = std::time::Instant::now();
        }
    }

    if sink.total > 0 && sink.written < sink.total {
        return Err(Fail::Transient(format!(
            "Download truncated at {} of {} bytes",
            sink.written, sink.total
        )));
    }

    Ok(())
}

fn verify_file_checksum(path: &std::path::Path, expected: &str) -> Result<bool, String> {
    let mut file =
        std::fs::File::open(path).map_err(|e| format!("Failed to open file for checksum: {e}"))?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|e| format!("Failed to hash file: {e}"))?;
    let hash = format!("{:x}", hasher.finalize());
    Ok(hash == expected.to_ascii_lowercase())
}

fn sidecar(image_path: &std::path::Path, suffix: &str) -> std::path::PathBuf {
    let mut name = image_path.as_os_str().to_owned();
    name.push(suffix);
    std::path::PathBuf::from(name)
}

/// Sidecar for a *complete*, checksum-verified image.
fn marker_path(image_path: &std::path::Path) -> std::path::PathBuf {
    sidecar(image_path, ".cache")
}

/// Sidecar for an *incomplete* download: how far it got, and for which release.
fn part_path(image_path: &std::path::Path) -> std::path::PathBuf {
    sidecar(image_path, ".part")
}

fn read_marker_file(path: &std::path::Path) -> Option<(String, u64)> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut parts = content.split_whitespace();
    let sha = parts.next()?.to_string();
    let size = parts.next()?.parse::<u64>().ok()?;
    Some((sha, size))
}

fn read_marker(image_path: &std::path::Path) -> Option<(String, u64)> {
    read_marker_file(&marker_path(image_path))
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

#[derive(Default)]
struct FlashTrace {
    image_bytes: u64,
    compressed: bool,
    decompress_ms: Option<u64>,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn flash_image(
    app_handle: AppHandle,
    state: tauri::State<'_, FlasherState>,
    job_id: String,
    image_path: String,
    target_disk: String,
    wifi: Option<WifiConfig>,
    hostname: Option<String>,
    tailscale: Option<TailscaleConfig>,
    access: Option<flasher_core::AccessConfig>,
    init_format: flasher_core::InitFormat,
) -> Result<(), String> {
    let config = ProvisionConfig {
        hostname,
        wifi,
        tailscale,
        access,
        init_format,
    };
    let facts = telemetry::describe_config(&config);
    let phases = Arc::new(std::sync::Mutex::new(telemetry::PhaseLog::default()));
    let cancelled = state.user_cancel.clone();
    let started = std::time::Instant::now();
    let mut trace = FlashTrace::default();

    let result = flash_image_inner(
        &app_handle,
        &state,
        &config,
        image_path,
        target_disk,
        phases.clone(),
        &mut trace,
    )
    .await;

    let end = std::time::Instant::now();
    let duration_ms = started.elapsed().as_millis() as u64;
    let log = phases.lock().expect("phase log poisoned");
    let write_ms = log.duration_ms("write", end);
    let outcome = match &result {
        Ok(()) => "success",
        Err(_) if cancelled.load(Ordering::SeqCst) => "cancelled",
        Err(_) => "failed",
    };
    let failed_at = result.as_ref().err().map(|_| {
        log.last_stage().unwrap_or(if trace.decompress_ms.is_some() {
            "launch"
        } else if trace.compressed {
            "decompress"
        } else {
            "prepare"
        })
    });

    telemetry::record(
        &app_handle,
        &telemetry::FlashEvent {
            envelope: telemetry::Envelope::new(
                "flash",
                job_id,
                &app_handle,
                outcome,
                duration_ms,
                result.as_ref().err().cloned(),
            ),
            config: facts,
            image_bytes: trace.image_bytes,
            compressed: trace.compressed,
            verified: log.saw("verify"),
            failed_at,
            decompress_ms: trace.decompress_ms,
            write_ms,
            verify_ms: log.duration_ms("verify", end),
            customize_ms: log.duration_ms("customize", end),
            write_mbps: write_ms.and_then(|ms| telemetry::mbps(trace.image_bytes, ms)),
            diag: (result.is_err() && !log.diag.is_empty()).then(|| log.diag.clone()),
        },
    );

    result
}

#[tauri::command]
pub fn reveal_event_log(app_handle: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt as _;
    let path = telemetry::log_path(&app_handle)?;
    if !path.exists() {
        std::fs::write(&path, "").map_err(|e| format!("Failed to create log: {e}"))?;
    }
    app_handle
        .opener()
        .reveal_item_in_dir(&path)
        .map_err(|e| format!("Failed to reveal log: {e}"))
}

async fn flash_image_inner(
    app_handle: &AppHandle,
    state: &tauri::State<'_, FlasherState>,
    config: &ProvisionConfig,
    image_path: String,
    target_disk: String,
    phases: Arc<std::sync::Mutex<telemetry::PhaseLog>>,
    trace: &mut FlashTrace,
) -> Result<(), String> {
    validate_disk_path(&target_disk)?;
    let src_path = validate_image_file(std::path::Path::new(&image_path))?;

    // Re-validate the target is still a removable external device.
    let devices = list_block_devices().await?;
    let Some(card) = devices.iter().find(|d| d.path == target_disk) else {
        return Err(format!(
            "Target disk {target_disk} is not a valid removable device"
        ));
    };
    let card_size = card.size;
    let card_label = format!("{} ({})", card.name, card.size_human);

    state.user_cancel.store(false, Ordering::SeqCst);
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();

    let temp_dir = private_temp_dir()?;

    // Decompress only if compressed; raw .img is written directly (local files).
    let lower = src_path.to_string_lossy().to_ascii_lowercase();
    trace.compressed = lower.ends_with(".xz") || lower.ends_with(".gz");
    let (write_path, is_temp) = if trace.compressed {
        check_available_space(&temp_dir, MIN_TEMP_SPACE_BYTES)?;
        let dst = temp_dir.join("aircast-image.img");
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
        }
        (dst, true)
    } else {
        (src_path.clone(), false)
    };

    // For an uncompressed image the size is known now, so a card that cannot
    // hold it fails before the elevation prompt instead of after it.
    if !trace.compressed {
        let total = std::fs::metadata(&write_path).map(|m| m.len()).unwrap_or(0);
        fits_on_card(total, card_size, &card_label)?;
    }

    let provision_b64 = {
        use base64::Engine as _;
        let json = serde_json::to_vec(config)
            .map_err(|e| format!("Failed to encode provision config: {e}"))?;
        base64::engine::general_purpose::STANDARD.encode(json)
    };

    // Launch the elevated helper FIRST so it opens and probes the device while
    // the image decompresses: a bad device/reader fails in seconds, not after a
    // 50-second decompress. The ready-file handshake tells the helper when the
    // image is complete ("ok") or the preparation died ("abort"). On failure the
    // helper sets `cancel`, which aborts the concurrent decompress.
    let ready_file = temp_dir.join(format!("aircast-image-ready-{}.txt", randbits()));
    let _ = std::fs::remove_file(&ready_file);

    let helper = tokio::spawn(run_elevated_flash(
        app_handle.clone(),
        target_disk.clone(),
        write_path.clone(),
        ready_file.clone(),
        provision_b64,
        phases,
        cancel.clone(),
    ));

    let prepared = if trace.compressed {
        let unpack_started = std::time::Instant::now();
        decompress(app_handle, &src_path, &write_path, &lower, cancel.clone())
            .await
            .map(|_| {
                trace.decompress_ms = Some(unpack_started.elapsed().as_millis() as u64);
            })
    } else {
        Ok(())
    };

    let prepared = prepared.and_then(|()| {
        let total = std::fs::metadata(&write_path).map(|m| m.len()).unwrap_or(0);
        trace.image_bytes = total;
        fits_on_card(total, card_size, &card_label).map(|()| total)
    });

    let result = match prepared {
        Ok(total) => {
            let _ = std::fs::write(&ready_file, "ok");
            emit_flash_progress(app_handle, FlashPhase::Writing, 0, total, 0.0, 0);
            helper
                .await
                .unwrap_or_else(|e| Err(format!("Elevated flash task failed: {e}")))
        }
        Err(prep_error) => {
            let _ = std::fs::write(&ready_file, "abort");
            // The helper may still be sitting behind an unanswered Touch ID/UAC
            // prompt, where it cannot see the abort file at all. Report the real
            // failure rather than blocking on the user dismissing a dialog for a
            // flash that has already lost.
            let helper_result = match tokio::time::timeout(HELPER_ABORT_GRACE, helper).await {
                Ok(joined) => {
                    joined.unwrap_or_else(|e| Err(format!("Elevated flash task failed: {e}")))
                }
                Err(_) => Ok(()),
            };
            // If the helper died on its own (device error, declined elevation),
            // that is the root cause and the decompress abort is collateral.
            match helper_result {
                Err(e) if !e.contains("image preparation aborted") => Err(e),
                _ => Err(prep_error),
            }
        }
    };

    let _ = std::fs::remove_file(&ready_file);
    if is_temp {
        let _ = std::fs::remove_file(&write_path);
    }
    result
}

/// The helper's stage strings, as `'static` labels for the phase log.
fn stage_label(stage: &str) -> Option<&'static str> {
    match stage {
        "device" => Some("device"),
        "write" => Some("write"),
        "verify" => Some("verify"),
        "customize" => Some("customize"),
        _ => None,
    }
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
    app_handle: AppHandle,
    target_disk: String,
    image: std::path::PathBuf,
    ready_file: std::path::PathBuf,
    provision_b64: String,
    phases: Arc<std::sync::Mutex<telemetry::PhaseLog>>,
    cancel: Arc<AtomicBool>,
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
        let mut speed = FlashSpeed::default();
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
                            if let Some(d) = v.get("diag").and_then(|x| x.as_str()) {
                                if let Ok(mut log) = phases.lock() {
                                    log.diag.push(d.to_string());
                                }
                                continue;
                            }
                            let stage = v.get("stage").and_then(|x| x.as_str());
                            let done = v.get("done").and_then(|x| x.as_u64()).unwrap_or(0);
                            let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0);
                            if let Some(stage) = stage {
                                if let (Some(label), Ok(mut log)) = (stage_label(stage), phases.lock())
                                {
                                    log.mark(label, done);
                                }
                                if let Some(phase) = phase_from_stage(stage) {
                                    let percent = if total > 0 {
                                        (done as f64 / total as f64) * 100.0
                                    } else {
                                        0.0
                                    };
                                    let bps = speed.sample(phase, done);
                                    emit_flash_progress(&app, phase, done, total, percent, bps);
                                }
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

    // The provision blob holds secrets (device password, Tailscale key), so it
    // goes to the helper as a 0600 file path — never on the command line, where
    // any local user could read it via `ps`.
    sweep_stale_provision_files();
    let provision_file =
        std::env::temp_dir().join(format!("{PROVISION_FILE_PREFIX}{}.b64", randbits()));
    write_secret_file(&provision_file, &provision_b64)?;
    let provision_arg = provision_file.to_string_lossy().to_string();

    let launch = launch_helper(
        &exe,
        &target_disk,
        &image,
        &progress_file,
        &provision_arg,
        &ready_file,
    )
    .await;
    let _ = std::fs::remove_file(&provision_file);

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

    let out = match launch {
        Ok(true) => Ok(()),
        Ok(false) => Err(last_error.unwrap_or_else(|| {
            "Elevated flash failed (authorization declined or write error)".to_string()
        })),
        Err(e) => Err(last_error.unwrap_or(e)),
    };
    // A helper that died while the main process is still decompressing must
    // abort that decompress: its cancel check is the only thing watching us.
    if out.is_err() {
        cancel.store(true, Ordering::SeqCst);
    }
    out
}

const PROVISION_FILE_PREFIX: &str = "aircast-provision-";

/// Writes `contents` to `path` as a `0600` file (owner-only on Unix), replacing
/// any existing file. Used for the secret-bearing provision blob.
fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write as _;
    let _ = std::fs::remove_file(path);
    let mut f = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        }
        // Windows has no mode bits here; the file lands in the user's temp dir
        // (readable by the same user and admins). The argv exposure is what
        // mattered — a locked-down ACL would need the windows-acl crate.
        #[cfg(not(unix))]
        {
            std::fs::File::create(path)
        }
    }
    .map_err(|e| format!("Failed to create provision file: {e}"))?;
    f.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write provision file: {e}"))?;
    Ok(())
}

/// No live flash runs anywhere near this long, so a provision blob older than
/// this is an orphan from a crashed flasher.
const STALE_PROVISION_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Best-effort removal of orphaned provision blobs in the temp dir, left by a
/// flasher that crashed between writing the file and cleaning it up. Only files
/// older than [`STALE_PROVISION_AGE`] are removed, so a concurrently-running
/// flasher's in-flight blob is never deleted.
fn sweep_stale_provision_files() {
    sweep_provision_files_in(&std::env::temp_dir(), STALE_PROVISION_AGE);
}

fn sweep_provision_files_in(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(PROVISION_FILE_PREFIX)
        {
            continue;
        }
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if too_old {
            let _ = std::fs::remove_file(entry.path());
        }
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
    provision_file: &str,
    ready_file: &std::path::Path,
) -> Result<bool, String> {
    use crate::macos_auth::Authorization;

    let exe = exe.to_string_lossy().to_string();
    let device = target_disk.to_string();
    let image = image.to_string_lossy().to_string();
    let pf = progress_file.to_string_lossy().to_string();
    let provision = provision_file.to_string();
    let ready = ready_file.to_string_lossy().to_string();

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
                "--provision-file",
                &provision,
                "--image-ready-file",
                &ready,
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
    provision_file: &str,
    ready_file: &std::path::Path,
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
            "--provision-file",
            provision_file,
            "--image-ready-file",
            &ready_file.to_string_lossy(),
        ])
        .status()
        .await
        .map_err(|e| format!("Failed to launch elevated flasher via pkexec: {e}"))?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
pub(crate) fn hidden_command(program: &str) -> tokio::process::Command {
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
    provision_file: &str,
    ready_file: &std::path::Path,
) -> Result<bool, String> {
    // Windows has no pkexec/Authorization-Services: re-exec this binary elevated
    // via PowerShell's Start-Process -Verb RunAs (UAC) and wait for it.
    let ps_cmd = format!(
        "$p = Start-Process -FilePath '{exe}' -ArgumentList \
         '--flash-helper','--device','{device}','--image','{image}','--progress-file','{pf}','--provision-file','{provision}','--image-ready-file','{ready}' \
         -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        exe = ps_escape(&exe.to_string_lossy()),
        device = ps_escape(target_disk),
        image = ps_escape(&image.to_string_lossy()),
        pf = ps_escape(&progress_file.to_string_lossy()),
        provision = ps_escape(provision_file),
        ready = ps_escape(&ready_file.to_string_lossy()),
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
    _ready_file: &std::path::Path,
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
    emit_flash_progress(app_handle, FlashPhase::Decompressing, 0, 0, 0.0, 0);

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
                    0,
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
            0,
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
    state.user_cancel.store(true, Ordering::SeqCst);
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
pub(crate) async fn wifi_device_macos() -> String {
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
    speed_bps: u64,
) {
    let _ = app_handle.emit(
        "flasher:flash-progress",
        FlashProgress {
            phase,
            bytes_processed,
            total_bytes,
            percent,
            speed_bps,
        },
    );
}

/// Bytes per second over the same one-second window the download uses, so both
/// progress panels report a rate the same way. Restarts on a phase change,
/// since write and verify are different speeds over the same byte count.
#[derive(Default)]
struct FlashSpeed {
    phase: Option<FlashPhase>,
    window_start: Option<std::time::Instant>,
    window_bytes: u64,
    bps: u64,
}

impl FlashSpeed {
    fn sample(&mut self, phase: FlashPhase, done: u64) -> u64 {
        if self.phase != Some(phase) || done < self.window_bytes {
            self.phase = Some(phase);
            self.window_start = Some(std::time::Instant::now());
            self.window_bytes = done;
            self.bps = 0;
            return 0;
        }
        let elapsed = self
            .window_start
            .map(|start| start.elapsed())
            .unwrap_or_default();
        if elapsed >= SPEED_WINDOW {
            self.bps = ((done - self.window_bytes) as f64 / elapsed.as_secs_f64()) as u64;
            self.window_start = Some(std::time::Instant::now());
            self.window_bytes = done;
        }
        self.bps
    }
}

fn format_bytes(bytes: u64) -> String {
    let unit = byte_unit::Byte::from_u64(bytes);
    let adjusted = unit.get_appropriate_unit(byte_unit::UnitType::Decimal);
    format!("{adjusted:.1}")
}

// ---------------------------------------------------------------------------
// Command: SSH public keys (auto-detect + read a picked file)
// ---------------------------------------------------------------------------

#[derive(serde::Serialize)]
pub struct SshPublicKey {
    pub label: String,
    pub contents: String,
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

/// The operator's `~/.ssh/*.pub` public keys, so the UI can offer them instead
/// of making the user cat-and-paste. Best-effort: returns empty on any error.
#[tauri::command]
pub fn detect_ssh_keys() -> Vec<SshPublicKey> {
    match home_dir() {
        Some(home) => detect_ssh_keys_in(&home.join(".ssh")),
        None => Vec::new(),
    }
}

fn detect_ssh_keys_in(ssh: &std::path::Path) -> Vec<SshPublicKey> {
    let Ok(entries) = std::fs::read_dir(ssh) else {
        return Vec::new();
    };
    let mut keys: Vec<SshPublicKey> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("pub"))
        .filter_map(|e| {
            let contents = std::fs::read_to_string(e.path()).ok()?.trim().to_string();
            (!contents.is_empty()).then(|| SshPublicKey {
                label: e.file_name().to_string_lossy().to_string(),
                contents,
            })
        })
        .collect();
    keys.sort_by(|a, b| a.label.cmp(&b.label));
    keys
}

const MAX_PUBKEY_BYTES: u64 = 64 * 1024;

/// Reads a public-key file the user picked in the file dialog. Capped so a
/// misclick on a huge file can't be slurped into memory.
#[tauri::command]
pub fn read_public_key(path: String) -> Result<String, String> {
    let meta = std::fs::metadata(&path).map_err(|e| format!("Cannot read key file: {e}"))?;
    if meta.len() > MAX_PUBKEY_BYTES {
        return Err("That file is too large to be an SSH public key.".to_string());
    }
    let contents = std::fs::read_to_string(&path).map_err(|e| format!("Cannot read key file: {e}"))?;
    Ok(contents.trim().to_string())
}

/// What a public key *is*, for a human about to disable password login with it.
/// The comment (`user@host`) is the part people recognise; the fingerprint is
/// what `ssh-keygen -lf` prints, so it can be compared against the key they
/// believe they hold. Returns `None` for anything that is not a key.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SshKeyIdentity {
    pub algorithm: String,
    pub comment: String,
    pub fingerprint: String,
}

#[tauri::command]
pub fn identify_public_key(key: String) -> Option<SshKeyIdentity> {
    use base64::Engine;
    use sha2::{Digest, Sha256};

    let mut parts = key.split_whitespace();
    let algorithm = parts.next()?.to_string();
    let blob = parts.next()?;
    let comment = parts.collect::<Vec<_>>().join(" ");

    let decoded = base64::engine::general_purpose::STANDARD
        .decode(blob)
        .ok()?;
    if decoded.is_empty() {
        return None;
    }

    // OpenSSH prints the SHA256 digest base64'd with the padding stripped.
    let digest = Sha256::digest(&decoded);
    let encoded = base64::engine::general_purpose::STANDARD.encode(digest);

    Some(SshKeyIdentity {
        algorithm,
        comment,
        fingerprint: format!("SHA256:{}", encoded.trim_end_matches('=')),
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Read one HTTP request head. Reads until the blank line rather than
    /// assuming the whole head lands in a single TCP segment.
    async fn read_request(sock: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;

        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            if sock.read(&mut byte).await.unwrap() == 0 {
                break;
            }
            head.push(byte[0]);
        }
        String::from_utf8_lossy(&head).into_owned()
    }

    /// Serves BODY in two halves: the first connection dies mid-stream (what a
    /// Wi-Fi/cellular switch does to an in-flight download), the second must
    /// arrive with `Range: bytes=<half>-` and gets a 206 with the remainder.
    async fn flaky_server(body: &'static [u8]) -> (String, tokio::task::JoinHandle<bool>) {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let half = body.len() / 2;

        let handle = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            read_request(&mut first).await;
            first
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            first.write_all(&body[..half]).await.unwrap();
            first.flush().await.unwrap();
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            let asked_for_resume = read_request(&mut second)
                .await
                .contains(&format!("bytes={half}-"));
            second
                .write_all(
                    format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {}-{}/{}\r\n\r\n",
                        body.len() - half,
                        half,
                        body.len() - 1,
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second.write_all(&body[half..]).await.unwrap();
            second.flush().await.unwrap();
            asked_for_resume
        });

        (url, handle)
    }

    /// Serves BODY, dying mid-stream on the first connection exactly like
    /// [`flaky_server`], but answering the resume request with `status` and
    /// `content_range` — the shapes a proxy or range-ignoring cache returns.
    async fn hostile_resume_server(
        body: &'static [u8],
        status: &'static str,
        content_range: Option<&'static str>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/image", listener.local_addr().unwrap());
        let half = body.len() / 2;

        let handle = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            read_request(&mut first).await;
            first
                .write_all(
                    format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).as_bytes(),
                )
                .await
                .unwrap();
            first.write_all(&body[..half]).await.unwrap();
            first.flush().await.unwrap();
            drop(first);

            let (mut second, _) = listener.accept().await.unwrap();
            read_request(&mut second).await;
            let range = content_range
                .map(|r| format!("Content-Range: {r}\r\n"))
                .unwrap_or_default();
            second
                .write_all(
                    format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{range}\r\n",
                        body.len()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
            second.write_all(body).await.unwrap();
            second.flush().await.unwrap();
        });

        (url, handle)
    }

    async fn test_sink(name: &str) -> (std::path::PathBuf, Sink) {
        let path = isolated_dir(name).join("image.img.xz");
        let sink = Sink {
            file: tokio::fs::File::create(&path).await.unwrap(),
            hasher: Sha256::new(),
            written: 0,
            total: 0,
        };
        (path, sink)
    }

    async fn finish(sink: &mut Sink) -> Vec<u8> {
        sink.file.flush().await.unwrap();
        sink.hasher.clone().finalize().to_vec()
    }

    #[tokio::test]
    async fn download_resumes_after_the_connection_drops() {
        const BODY: &[u8] = b"aircast-image-payload-that-spans-two-http-responses";

        let (url, server) = flaky_server(BODY).await;
        let (path, mut sink) = test_sink("resume").await;
        let cancel = Arc::new(AtomicBool::new(false));
        let mut stats = DownloadStats::default();

        let first = stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats).await;
        assert!(
            matches!(first, Err(Fail::Transient(_))),
            "a mid-stream drop must be retryable, not fatal"
        );
        assert!(sink.written > 0 && sink.written < BODY.len() as u64);

        stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats)
            .await
            .map_err(|e| match e {
                Fail::Transient(m) | Fail::Fatal(m) => m,
            })
            .expect("the retry must complete the download");

        assert!(server.await.unwrap(), "the retry must send a Range header");
        assert_eq!(sink.written, BODY.len() as u64);
        assert_eq!(
            finish(&mut sink).await,
            Sha256::digest(BODY).to_vec(),
            "the hash must span both halves exactly once"
        );
        assert_eq!(std::fs::read(&path).unwrap(), BODY, "bytes on disk");
        assert_eq!(stats.restarts, 0, "a clean resume must not restart");
        assert!(stats.resumed_bytes > 0, "resumed bytes must be recorded");
    }

    /// A 206 whose Content-Range does not start where the partial file ends
    /// must restart, not append. Appending would corrupt the file and only be
    /// caught by the final checksum, after the whole image was downloaded.
    #[tokio::test]
    async fn download_restarts_when_the_server_resumes_at_the_wrong_offset() {
        const BODY: &[u8] = b"aircast-image-payload-that-spans-two-http-responses";

        for (name, status, range) in [
            ("wrong-offset", "206 Partial Content", Some("bytes 0-49/50")),
            ("no-range", "206 Partial Content", None),
            ("ignores-range", "200 OK", None),
        ] {
            let (url, server) = hostile_resume_server(BODY, status, range).await;
            let (path, mut sink) = test_sink(name).await;
            let cancel = Arc::new(AtomicBool::new(false));
            let mut stats = DownloadStats::default();

            let _ = stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats).await;
            assert!(sink.written > 0 && sink.written < BODY.len() as u64);

            stream_into(&mut |_| {}, &cancel, &url, &mut sink, &mut stats)
                .await
                .map_err(|e| match e {
                    Fail::Transient(m) | Fail::Fatal(m) => m,
                })
                .unwrap_or_else(|e| panic!("{name}: restart must complete: {e}"));

            server.await.unwrap();
            assert_eq!(stats.restarts, 1, "{name}: must restart exactly once");
            assert_eq!(sink.written, BODY.len() as u64, "{name}: length");
            assert_eq!(
                finish(&mut sink).await,
                Sha256::digest(BODY).to_vec(),
                "{name}: the hash must cover the restarted stream only"
            );
            assert_eq!(std::fs::read(&path).unwrap(), BODY, "{name}: bytes on disk");
        }
    }

    #[tokio::test]
    async fn cancelling_skips_the_request_and_cuts_the_backoff_short() {
        let cancel = Arc::new(AtomicBool::new(true));
        let (_path, mut sink) = test_sink("cancel").await;
        let mut stats = DownloadStats::default();

        let started = std::time::Instant::now();
        let result = stream_into(
            &mut |_| {},
            &cancel,
            "http://127.0.0.1:1/never-listens",
            &mut sink,
            &mut stats,
        )
        .await;
        assert!(
            matches!(result, Err(Fail::Fatal(ref m)) if m.contains("cancelled")),
            "a cancelled download must not issue the request at all"
        );

        assert!(
            !sleep_unless_cancelled(&cancel, Duration::from_secs(30)).await,
            "backoff must report the cancellation"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "cancel must not wait out the connect timeout or the backoff"
        );
    }

    #[test]
    fn content_range_start_reads_the_offset() {
        let parse = |v: &str| {
            let mut h = reqwest::header::HeaderMap::new();
            h.insert(reqwest::header::CONTENT_RANGE, v.parse().unwrap());
            content_range_start(&h)
        };
        assert_eq!(parse("bytes 1024-2047/2048"), Some(1024));
        assert_eq!(parse("bytes 0-0/1"), Some(0));
        assert_eq!(parse("items 1024-2047/2048"), None);
        assert_eq!(parse("bytes */2048"), None);
        assert_eq!(content_range_start(&reqwest::header::HeaderMap::new()), None);
    }

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff(1), BACKOFF_BASE);
        assert_eq!(backoff(2), BACKOFF_BASE * 2);
        assert_eq!(backoff(3), BACKOFF_BASE * 4);
        assert_eq!(backoff(99), BACKOFF_CAP, "must not overflow or run away");
    }

    /// The card has to hold the *decompressed* image. v0.3.2 is 586 MB
    /// compressed and 2.97 GB unpacked, so a check against the download size
    /// would clear a 1 GB card that cannot possibly hold it.
    #[test]
    fn card_check_uses_the_decompressed_size() {
        const UNCOMPRESSED: u64 = 2_969_567_232;
        const COMPRESSED: u64 = 586_187_284;
        const ONE_GB_CARD: u64 = 1_000_000_000;

        // The floor has to sit above a real image and above the cards that
        // comparing against the *download* size alone would wrongly clear.
        const _: () = assert!(MIN_CARD_BYTES > UNCOMPRESSED);
        const _: () = assert!(COMPRESSED < ONE_GB_CARD && ONE_GB_CARD < MIN_CARD_BYTES);

        assert!(
            fits_on_card(UNCOMPRESSED, ONE_GB_CARD, "SD (1 GB)").is_err(),
            "a 1 GB card cannot hold a 2.97 GB image"
        );
        assert!(
            fits_on_card(UNCOMPRESSED, 4_000_000_000, "SD (4 GB)").is_ok(),
            "a real 4 GB card must not be rejected"
        );
        assert!(
            fits_on_card(UNCOMPRESSED, 0, "SD (unknown)").is_ok(),
            "an unknown card size must not block the flash"
        );
    }

    #[tokio::test]
    async fn a_failed_download_resumes_from_its_partial_file() {
        const BODY: &[u8] = b"aircast-image-payload-that-spans-two-http-responses";
        let half = BODY.len() / 2;
        let checksum = format!("{:x}", Sha256::digest(BODY));

        let dir = isolated_dir("partial");
        let image = dir.join("image.img.xz");
        std::fs::write(&image, &BODY[..half]).unwrap();
        std::fs::write(part_path(&image), format!("{checksum} {half}")).unwrap();

        let mut sink = resume_partial(&image, &checksum)
            .await
            .expect("a partial from this release must be resumable");
        assert_eq!(sink.written, half as u64);

        sink.hasher.update(&BODY[half..]);
        assert_eq!(
            format!("{:x}", sink.hasher.clone().finalize()),
            checksum,
            "the reseeded hash must match hashing the whole body once"
        );

        assert!(
            resume_partial(&image, &"0".repeat(64)).await.is_none(),
            "a partial from a different release must not be resumed"
        );
    }

    #[test]
    fn percent_never_exceeds_one_hundred() {
        assert_eq!(percent_of(50, 100), 50.0);
        assert_eq!(percent_of(0, 0), 0.0);
        assert_eq!(
            percent_of(120, 100),
            100.0,
            "an over-long body must not render as 120%"
        );
    }

    #[test]
    fn validate_disk_path_rejects_injection() {
        assert!(validate_disk_path("/dev/disk2; rm -rf /").is_err());
        assert!(validate_disk_path("/dev/disk2$(whoami)").is_err());
        assert!(validate_disk_path("../../etc/passwd").is_err());
        assert!(validate_disk_path("").is_err());
    }

    // A unique temp subdir per test *and* per process: a previous run whose
    // cleanup lost the race would otherwise leave a file behind and fail the
    // next run's `create_new`.
    fn isolated_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("aircast-flasher-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_secret_file_round_trips_contents() {
        let dir = isolated_dir("roundtrip");
        let path = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&path, "secret-blob").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret-blob");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_secret_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = isolated_dir("mode");
        let path = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&path, "secret-blob").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        // The secret-bearing blob must not be world/group readable.
        assert_eq!(mode & 0o777, 0o600, "provision file must be 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_removes_stale_provision_files() {
        let dir = isolated_dir("sweep-stale");
        let stale = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&stale, "x").unwrap();
        // Zero threshold: every matching file counts as stale.
        sweep_provision_files_in(&dir, std::time::Duration::ZERO);
        assert!(!stale.exists(), "sweep must remove orphaned provision files");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detect_ssh_keys_reads_pub_files_only() {
        let dir = isolated_dir("ssh-detect");
        std::fs::write(dir.join("id_ed25519.pub"), "  ssh-ed25519 AAAAKEY me@host  \n").unwrap();
        std::fs::write(dir.join("id_ed25519"), "PRIVATE KEY").unwrap();
        std::fs::write(dir.join("empty.pub"), "   \n").unwrap();

        let keys = detect_ssh_keys_in(&dir);

        assert_eq!(keys.len(), 1, "only non-empty .pub files, never private keys");
        assert_eq!(keys[0].label, "id_ed25519.pub");
        assert_eq!(keys[0].contents, "ssh-ed25519 AAAAKEY me@host");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn identifies_a_public_key_the_way_ssh_keygen_does() {
        // ssh-keygen -lf on this key prints exactly this fingerprint.
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ3z5r1sZ0K7DxHhqTn7lVXJHkiqrxKZ+i0Ck0KFqkkn operator@base";
        let id = identify_public_key(key.to_string()).expect("a key");
        assert_eq!(id.algorithm, "ssh-ed25519");
        assert_eq!(id.comment, "operator@base");
        // Verified against `ssh-keygen -lf` on this exact key.
        assert_eq!(id.fingerprint, "SHA256:6ToLTRFTRhCceqBA4tBy84TESMsJViBkYltFaKXXP20");
    }

    #[test]
    fn a_key_with_no_comment_still_identifies() {
        let id = identify_public_key(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJ3z5r1sZ0K7DxHhqTn7lVXJHkiqrxKZ+i0Ck0KFqkkn".into(),
        )
        .expect("a key");
        assert_eq!(id.comment, "");
    }

    #[test]
    fn rubbish_is_not_a_key() {
        assert!(identify_public_key("hello".into()).is_none());
        assert!(identify_public_key("ssh-ed25519 not-base64!!".into()).is_none());
        assert!(identify_public_key(String::new()).is_none());
    }

    #[test]
    fn read_public_key_trims_and_caps_size() {
        let dir = isolated_dir("ssh-read");
        let ok = dir.join("k.pub");
        std::fs::write(&ok, "ssh-ed25519 AAAA me@host\n").unwrap();
        assert_eq!(read_public_key(ok.to_string_lossy().into()).unwrap(), "ssh-ed25519 AAAA me@host");

        let big = dir.join("big.pub");
        std::fs::write(&big, vec![b'a'; (MAX_PUBKEY_BYTES + 1) as usize]).unwrap();
        assert!(read_public_key(big.to_string_lossy().into()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_keeps_recent_provision_files() {
        let dir = isolated_dir("sweep-recent");
        let fresh = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&fresh, "x").unwrap();
        // A just-written file is younger than the threshold: a concurrent
        // flash's in-flight blob must survive.
        sweep_provision_files_in(&dir, std::time::Duration::from_secs(3600));
        assert!(fresh.exists(), "sweep must not delete an in-flight provision file");
        let _ = std::fs::remove_dir_all(&dir);
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
