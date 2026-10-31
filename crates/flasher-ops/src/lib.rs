pub mod cloud;
pub mod devices;
pub mod download;
pub mod flash;
pub mod helper;
#[cfg(target_os = "macos")]
mod macos_auth;
pub mod paths;
pub mod releases;
pub mod sshkeys;
pub mod tailnet;
pub mod tang;
pub mod usbboard;
pub mod wifi;

mod types;
mod validation;

pub use types::*;
pub use validation::*;

pub use flasher_core::{
    AccessConfig, EncryptionConfig, InitFormat, ProvisionConfig, SshMode, TailscaleConfig,
    TangServer, WifiConfig,
};

use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

pub(crate) const PROGRESS_THROTTLE_MS: u128 = 100;
pub(crate) const MIN_TEMP_SPACE_BYTES: u64 = 4_500_000_000;

pub(crate) const MIN_CARD_BYTES: u64 = 3_500_000_000;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const STALL_TIMEOUT: Duration = Duration::from_secs(20);
pub(crate) const SPEED_WINDOW: Duration = Duration::from_secs(1);
pub(crate) const BACKOFF_BASE: Duration = Duration::from_secs(1);
pub(crate) const BACKOFF_CAP: Duration = Duration::from_secs(30);
pub(crate) const CANCEL_POLL: Duration = Duration::from_millis(100);

pub(crate) const STALLED_ATTEMPTS: usize = 6;

pub(crate) const MAX_RETRIES: usize = 100;

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

pub fn format_bytes(bytes: u64) -> String {
    let unit = byte_unit::Byte::from_u64(bytes);
    let adjusted = unit.get_appropriate_unit(byte_unit::UnitType::Decimal);
    format!("{adjusted:.1}")
}

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

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(target_os = "windows")]
pub(crate) fn hidden_command(program: &str) -> tokio::process::Command {
    let mut cmd = tokio::process::Command::new(program);
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}
