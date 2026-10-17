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

/// One Tang server the device can unlock against: its URL plus a captured
/// advertisement so first-boot enrolment needs no network.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TangServer {
    /// e.g. `http://10.0.0.5`. Must be reachable from the device over plain IP;
    /// the Tang protocol needs no secure channel.
    pub url: String,
    /// The Tang advertisement (`GET <url>/adv`), verbatim JSON.
    pub adv: String,
}

impl TangServer {
    fn is_complete(&self) -> bool {
        !self.url.trim().is_empty() && !self.adv.trim().is_empty()
    }
}

/// Disk encryption for the device's data partition (settings, credentials,
/// tailnet state). The partition is LUKS2, unlocked at every boot by Clevis
/// against one or more Tang servers (network-bound disk encryption) — so a
/// pulled card, or a powered device on a network without a Tang server, gives
/// nothing up. With several servers a `threshold` of them must answer (Clevis
/// `sss`), so one box being down does not lock the fleet out. The image enrols
/// this on first boot, then wipes the clear recovery key off the card.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionConfig {
    /// The Tang servers to bind to.
    pub tang_servers: Vec<TangServer>,
    /// How many of them must answer at unlock (1..=tang_servers.len()).
    pub threshold: u32,
    /// Break-glass recovery passphrase (LUKS slot 0). The operator keeps this;
    /// it is wiped off the card once first boot has enrolled the binding.
    pub recovery_key: String,
}

impl EncryptionConfig {
    /// True once encryption can actually be enrolled: a recovery key, at least
    /// one fully-captured server, and a threshold that fits the server count.
    pub fn is_complete(&self) -> bool {
        let servers = self.tang_servers.len() as u32;
        !self.recovery_key.trim().is_empty()
            && servers >= 1
            && self.tang_servers.iter().all(TangServer::is_complete)
            && self.threshold >= 1
            && self.threshold <= servers
    }
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
    #[serde(default)]
    pub encryption: Option<EncryptionConfig>,
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
