//! Flasher commands. Mirrors the proven aircast-web flasher: list releases,
//! enumerate removable block devices, stream-download with checksum verify,
//! decompress if needed, then run the open-once write → verify → customize
//! engine inside one elevated helper process (Touch ID on macOS / UAC on
//! Windows / pkexec on Linux). Cancellable via a shared flag.

pub mod devices;
pub mod download;
pub mod flash;
pub mod releases;
pub mod sshkeys;
pub mod wifi;

mod types;
mod validation;

pub(crate) use types::*;
pub(crate) use validation::*;

use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use flasher_core::{ProvisionConfig, TailscaleConfig, WifiConfig};

pub(crate) const PROGRESS_THROTTLE_MS: u128 = 100;
pub(crate) const MIN_TEMP_SPACE_BYTES: u64 = 4_500_000_000; // ~4.5 GB

/// Smallest card that can hold a decompressed aircast-lite image, used to warn
/// before the download when the release does not publish its uncompressed size.
/// v0.3.2 decompresses to 2.97 GB (`xz --robot -l`), so this rejects 2 GB cards
/// and clears every real 4 GB one. It is a floor, not the true requirement:
/// the exact check runs against the decompressed file in `flash_image_inner`.
pub(crate) const MIN_CARD_BYTES: u64 = 3_500_000_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const SPEED_WINDOW: Duration = Duration::from_secs(1);
pub(crate) const BACKOFF_BASE: Duration = Duration::from_secs(1);
pub(crate) const BACKOFF_CAP: Duration = Duration::from_secs(30);
pub(crate) const CANCEL_POLL: Duration = Duration::from_millis(100);

/// How many *consecutive* attempts may move zero bytes before the download is
/// declared dead. Any progress resets the count, so a long transfer over a link
/// that drops repeatedly still finishes; only a link that stops delivering
/// gives up.
pub(crate) const STALLED_ATTEMPTS: usize = 6;

/// Absolute backstop, since [`STALLED_ATTEMPTS`] resets on any progress at all:
/// a server dribbling one byte per connection would otherwise retry forever.
pub(crate) const MAX_RETRIES: usize = 100;

/// How long to wait for the elevated helper to notice an aborted preparation
/// before reporting the preparation's own error. Long enough for a helper that
/// is actually running to read the abort file, short enough that an unanswered
/// elevation prompt does not hold the UI.
pub(crate) const HELPER_ABORT_GRACE: Duration = Duration::from_secs(5);

pub(crate) fn http() -> &'static reqwest::Client {
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

// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn emit_flash_progress(
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
pub(crate) struct FlashSpeed {
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

pub(crate) fn format_bytes(bytes: u64) -> String {
    let unit = byte_unit::Byte::from_u64(bytes);
    let adjusted = unit.get_appropriate_unit(byte_unit::UnitType::Decimal);
    format!("{adjusted:.1}")
}

// ---------------------------------------------------------------------------

/// A directory of this test's own, so the ones that write files do not
/// collide when the suite runs in parallel.
#[cfg(test)]
pub(crate) fn isolated_dir(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "aircast-flasher-test-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

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
    fn releases_response_deserializes() {
        let json = r#"{"releases":[{"version":"v1.0.0","prerelease":false,"created_at":"2026-06-01T00:00:00Z","image":{"filename":"a.img.xz","extension":"img.xz","size":100,"download_url":"https://x/a.img.xz","checksum_url":"https://x/a.img.xz.sha256"}}]}"#;
        let r: ReleasesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(r.releases[0].version, "v1.0.0");
    }
}

/// Windows spawns a console window for every child process unless told not to.
/// Shared, because the flash helper and both Wi-Fi paths all shell out.
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
pub(crate) fn hidden_command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}
