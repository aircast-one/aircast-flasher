//! Provisioning configuration types. These cross the Tauri IPC boundary from the
//! UI, so they derive `Deserialize` (and `Serialize` for round-tripping/tests).

use serde::{Deserialize, Serialize};

/// WiFi credentials to provision onto the boot partition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiConfig {
    pub ssid: String,
    /// Plaintext passphrase. The plan derives the WPA-PSK (see [`crate::psk`]) so
    /// the plaintext is never written to the card.
    pub password: String,
    /// Two-letter WiFi regulatory domain, e.g. "US". Required or WiFi may stay
    /// rfkill-blocked on recent images.
    pub country: String,
}

/// Which first-boot mechanism the target image expects. Must match the image
/// base: Trixie (Nov 2025+) → cloud-init; Bookworm → firstrun.sh.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InitFormat {
    /// cloud-init: `user-data` + `network-config` on the boot partition.
    CloudInit,
    /// legacy: `firstrun.sh` wired via `cmdline.txt`.
    FirstRun,
}

/// What to provision. Any `None` field is left untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionConfig {
    pub hostname: Option<String>,
    pub wifi: Option<WifiConfig>,
    pub init_format: InitFormat,
}

impl ProvisionConfig {
    /// True when there is nothing to write (no hostname, no WiFi).
    pub fn is_empty(&self) -> bool {
        self.hostname.is_none() && self.wifi.is_none()
    }
}
