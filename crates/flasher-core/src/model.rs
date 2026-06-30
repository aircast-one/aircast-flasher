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

/// Control plane + pre-auth key for zero-touch Tailscale/Headscale enrollment.
/// Provisioned into aircastd's store so the device joins the tailnet on first
/// boot with no interactive sign-in.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleConfig {
    /// Login server URL, e.g. `https://headscale.example.com`. Empty for
    /// Tailscale's own coordination server.
    pub control_server: String,
    /// Reusable pre-auth key minted on the control server.
    pub auth_key: String,
}

/// How SSH should be set up on the device. Defaults are left untouched unless
/// the chosen mode is "effective" (see [`AccessConfig::is_effective`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SshMode {
    /// Public-key auth only; password login disabled.
    KeyOnly,
    /// Password auth, with the device password set from [`AccessConfig::password`].
    Password,
    /// SSH service disabled entirely.
    Disabled,
}

/// Device access (SSH) to provision. Replaces the image's default
/// `pi`/`raspberry` password login only when [`is_effective`](Self::is_effective)
/// is true, so a flash with no access input leaves the image defaults alone.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessConfig {
    pub ssh: SshMode,
    /// Authorized SSH public key, used in `KeyOnly` mode.
    #[serde(default)]
    pub authorized_key: Option<String>,
    /// New password for the `pi` account, used in `Password` mode.
    #[serde(default)]
    pub password: Option<String>,
}

impl AccessConfig {
    /// True when this config actually changes the device's access posture.
    /// `Disabled` always applies; `KeyOnly`/`Password` apply only once their
    /// required input (key / password) is present, so an untouched form is a
    /// no-op that preserves the image defaults.
    pub fn is_effective(&self) -> bool {
        match self.ssh {
            SshMode::Disabled => true,
            SshMode::KeyOnly => non_blank(&self.authorized_key),
            SshMode::Password => non_blank(&self.password),
        }
    }
}

fn non_blank(v: &Option<String>) -> bool {
    v.as_deref().is_some_and(|s| !s.trim().is_empty())
}

/// What to provision. Any `None` field is left untouched.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionConfig {
    pub hostname: Option<String>,
    pub wifi: Option<WifiConfig>,
    pub tailscale: Option<TailscaleConfig>,
    #[serde(default)]
    pub access: Option<AccessConfig>,
    pub init_format: InitFormat,
}

impl ProvisionConfig {
    /// True when there is nothing to write (no hostname, WiFi, Tailscale, or
    /// effective access change).
    pub fn is_empty(&self) -> bool {
        self.hostname.is_none()
            && self.wifi.is_none()
            && self.tailscale.is_none()
            && !self.access.as_ref().is_some_and(AccessConfig::is_effective)
    }
}
