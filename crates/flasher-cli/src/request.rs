use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use flasher_ops::flash::{FlashEvents, FlashTrace};
use flasher_ops::{
    AccessConfig, BlockDevice, DownloadProgress, EncryptionConfig, InitFormat, ProvisionConfig,
    Release, SshMode, TailscaleConfig, TangServer, WifiConfig,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::session;

#[derive(Debug, Default, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlashRequest {
    #[schemars(
        description = "The SD card to erase and write, exactly as list_cards reports it (e.g. /dev/disk4)."
    )]
    pub disk: String,
    #[schemars(
        description = "A local image file (.img, .img.xz or .img.gz). Leave out to download Aircast OS."
    )]
    pub image: Option<String>,
    #[schemars(
        description = "Aircast OS version to download (e.g. v0.3.6). Defaults to the newest stable release."
    )]
    pub release: Option<String>,
    #[schemars(description = "Release channel: stable, staging or development.")]
    pub channel: Option<String>,
    #[schemars(description = "Device hostname: lowercase letters, numbers and hyphens.")]
    pub hostname: Option<String>,
    #[schemars(
        description = "WiFi the device joins on first boot. Leave out for Ethernet or cellular."
    )]
    pub wifi: Option<WifiRequest>,
    #[schemars(
        description = "SSH login. Leave out to use the only key in ~/.ssh, or the image default login when there isn't exactly one."
    )]
    pub ssh: Option<SshRequest>,
    #[schemars(description = "Join a Tailscale or Headscale tailnet on first boot.")]
    pub tailscale: Option<TailnetRequest>,
    #[schemars(description = "Encrypt the data partition, unlocked by key servers at boot.")]
    pub encryption: Option<EncryptionRequest>,
    #[serde(default)]
    #[schemars(
        description = "No logs or video on the drone, tailnet-only access with an SSH key. Needs tailscale."
    )]
    pub fight_mode: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WifiRequest {
    pub ssid: String,
    #[serde(default)]
    #[schemars(description = "8–63 characters, or empty for an open network.")]
    pub password: String,
    #[schemars(description = "Two-letter regulatory country. Defaults to this computer's.")]
    pub country: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum SshChoice {
    Key,
    Password,
    Off,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SshRequest {
    pub mode: SshChoice,
    #[schemars(description = "Public key text, or a path to a .pub file. Mode key only.")]
    pub key: Option<String>,
    #[schemars(
        description = "Password for the pi account, at least 8 characters. Mode password only."
    )]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TailnetRequest {
    #[schemars(description = "Pre-auth key (tskey-auth-… or a Headscale key).")]
    pub auth_key: String,
    #[schemars(description = "Headscale control server URL. Leave out for Tailscale.")]
    pub control_server: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionRequest {
    #[serde(default)]
    #[schemars(description = "Tang key server URLs, e.g. http://10.0.0.5:7500.")]
    pub tang_servers: Vec<String>,
    #[schemars(description = "Key servers needed to unlock. Defaults to 1.")]
    pub threshold: Option<u32>,
    #[serde(default)]
    #[schemars(description = "Also unlock through Aircast Cloud. Needs a Cloud sign-in.")]
    pub cloud_unlock: bool,
    #[schemars(
        description = "Where to save the recovery key. Must not exist yet — it is the only copy."
    )]
    pub recovery_file: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FlashOutcome {
    pub disk: String,
    pub image: String,
    pub hostname: Option<String>,
    pub address: Option<String>,
    pub ssh: String,
    pub recovery_file: Option<String>,
}

const HOSTNAME_RULE: &str = "Use only lowercase letters, numbers and hyphens in the hostname";

fn hostname_valid(name: &str) -> bool {
    let bytes = name.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 63
        && bytes
            .iter()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || *b == b'-')
        && bytes.first() != Some(&b'-')
        && bytes.last() != Some(&b'-')
}

fn wifi_password_valid(password: &str) -> bool {
    password.is_empty()
        || (8..=63).contains(&password.len())
        || (password.len() == 64 && password.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn problems(request: &FlashRequest, detected_keys: usize) -> Vec<String> {
    let ssh_password = request
        .ssh
        .as_ref()
        .is_some_and(|ssh| ssh.mode == SshChoice::Password);
    let tailnet = request
        .tailscale
        .as_ref()
        .filter(|t| !t.auth_key.trim().is_empty());
    let encryption_servers = request
        .encryption
        .as_ref()
        .map(|e| e.tang_servers.len() as u32 + u32::from(e.cloud_unlock))
        .unwrap_or(0);

    [
        request
            .disk
            .trim()
            .is_empty()
            .then(|| "Choose the card to write (see `cards`)".to_string()),
        (request.image.is_some() && request.release.is_some())
            .then(|| "Give either a local image or a release, not both".to_string()),
        request
            .hostname
            .as_deref()
            .filter(|name| !hostname_valid(name))
            .map(|_| HOSTNAME_RULE.to_string()),
        request
            .wifi
            .as_ref()
            .filter(|w| w.ssid.trim().is_empty())
            .map(|_| "Enter the WiFi network name".to_string()),
        request
            .wifi
            .as_ref()
            .filter(|w| !wifi_password_valid(&w.password))
            .map(|_| "Make the WiFi password 8–63 characters".to_string()),
        request
            .ssh
            .as_ref()
            .filter(|s| s.mode == SshChoice::Password)
            .filter(|s| s.password.as_deref().unwrap_or("").len() < 8)
            .map(|_| "Make the device password at least 8 characters".to_string()),
        request
            .ssh
            .as_ref()
            .filter(|s| s.mode == SshChoice::Key && s.key.is_none())
            .map(|_| "Give the SSH public key, or a path to a .pub file".to_string()),
        request
            .tailscale
            .as_ref()
            .and_then(|t| t.control_server.as_deref())
            .filter(|url| !(url.starts_with("http://") || url.starts_with("https://")))
            .map(|_| "The control server must be an http(s):// URL".to_string()),
        (request.fight_mode && tailnet.is_none()).then(|| {
            "Fight mode needs a tailnet pre-auth key — the tailnet is the only way in".to_string()
        }),
        (request.fight_mode && ssh_password)
            .then(|| "Fight mode takes SSH keys only — use a key or turn SSH off".to_string()),
        (request.fight_mode && request.ssh.is_none() && detected_keys != 1).then(|| {
            "Fight mode needs an SSH key — pass one, or turn SSH off; the image's password login would stay on"
                .to_string()
        }),
        request
            .encryption
            .as_ref()
            .filter(|_| encryption_servers == 0)
            .map(|_| "Add a Tang key server or turn on Cloud unlock".to_string()),
        request
            .encryption
            .as_ref()
            .and_then(|e| e.threshold)
            .filter(|t| *t < 1 || *t > encryption_servers.max(1))
            .map(|_| {
                format!("Set the key servers needed to unlock between 1 and {encryption_servers}")
            }),
        request
            .encryption
            .as_ref()
            .filter(|e| e.recovery_file.trim().is_empty())
            .map(|_| "Give a file to save the recovery key to".to_string()),
        request
            .encryption
            .as_ref()
            .filter(|e| std::path::Path::new(&e.recovery_file).exists())
            .map(|e| {
                format!(
                    "{} already exists — the recovery key never overwrites a file",
                    e.recovery_file
                )
            }),
    ]
    .into_iter()
    .flatten()
    .collect()
}

pub const CARD_FLAGS_MIN_IMAGE: (u64, u64, u64) = (0, 3, 6);

fn version_at_least(version: &str, min: (u64, u64, u64)) -> bool {
    let core = version
        .trim_start_matches('v')
        .split(['-', '+'])
        .next()
        .unwrap_or("");
    let parts: Vec<u64> = core.split('.').filter_map(|n| n.parse().ok()).collect();
    match parts.as_slice() {
        [major, minor, patch] => (*major, *minor, *patch) >= min,
        _ => false,
    }
}

pub fn check_image(request: &FlashRequest, release: &Release) -> Result<(), String> {
    let needs_new_image = request.fight_mode || request.encryption.is_some();
    match needs_new_image && !version_at_least(&release.version, CARD_FLAGS_MIN_IMAGE) {
        true => {
            let (major, minor, patch) = CARD_FLAGS_MIN_IMAGE;
            Err(format!(
                "Fight mode and disk encryption need Aircast OS v{major}.{minor}.{patch} or newer — {} would ignore them. Pick a newer release (see `releases`)",
                release.version
            ))
        }
        false => Ok(()),
    }
}

pub async fn find_card(disk: &str) -> Result<BlockDevice, String> {
    let cards = flasher_ops::devices::list_block_devices().await?;
    cards
        .into_iter()
        .find(|card| card.path == disk)
        .ok_or_else(|| format!("{disk} isn't a removable SD card on this computer — see `cards`"))
}

pub async fn pick_release(
    release: Option<&str>,
    channel: Option<String>,
) -> Result<Release, String> {
    let releases = flasher_ops::releases::list_releases(channel)
        .await?
        .releases;
    match release {
        Some(version) => {
            let wanted = version.trim_start_matches('v');
            releases
                .into_iter()
                .find(|r| r.version.trim_start_matches('v') == wanted)
                .ok_or_else(|| format!("No Aircast OS release {version} — see `releases`"))
        }
        None => releases
            .iter()
            .find(|r| !r.prerelease)
            .or(releases.first())
            .cloned()
            .ok_or_else(|| "No Aircast OS release is available".to_string()),
    }
}

pub async fn download(
    release: &Release,
    cancel: &Arc<AtomicBool>,
    progress: &mut (dyn FnMut(&DownloadProgress) + Send),
) -> Result<String, String> {
    let cache = flasher_ops::paths::image_cache_dir()?;
    let mut stats = flasher_ops::download::DownloadStats::default();
    flasher_ops::download::download_image(
        &cache,
        &release.image.download_url,
        &release.image.checksum_url,
        cancel,
        progress,
        &mut stats,
    )
    .await
    .map(|result| result.image_path)
}

fn read_key(key: &str) -> Result<String, String> {
    let text = match std::path::Path::new(key).is_file() {
        true => flasher_ops::sshkeys::read_public_key(key.to_string())?,
        false => key.trim().to_string(),
    };
    flasher_ops::sshkeys::identify_public_key(text.clone())
        .map(|_| text)
        .ok_or_else(|| {
            "That isn't an SSH public key — it starts with ssh-ed25519, ssh-rsa, …".to_string()
        })
}

fn access(ssh: Option<&SshRequest>) -> Result<(Option<AccessConfig>, String), String> {
    match ssh {
        Some(SshRequest {
            mode: SshChoice::Off,
            ..
        }) => Ok((
            Some(AccessConfig {
                ssh: SshMode::Disabled,
                authorized_key: None,
                password: None,
            }),
            "off".to_string(),
        )),
        Some(SshRequest {
            mode: SshChoice::Password,
            password,
            ..
        }) => Ok((
            Some(AccessConfig {
                ssh: SshMode::Password,
                authorized_key: None,
                password: password.clone(),
            }),
            "password for pi".to_string(),
        )),
        Some(SshRequest {
            mode: SshChoice::Key,
            key,
            ..
        }) => read_key(key.as_deref().unwrap_or("")).map(|key| {
            (
                Some(AccessConfig {
                    ssh: SshMode::KeyOnly,
                    authorized_key: Some(key),
                    password: None,
                }),
                "SSH key".to_string(),
            )
        }),
        None => Ok(match flasher_ops::sshkeys::detect_ssh_keys().as_slice() {
            [only] => (
                Some(AccessConfig {
                    ssh: SshMode::KeyOnly,
                    authorized_key: Some(only.contents.clone()),
                    password: None,
                }),
                format!("SSH key · {}", only.label),
            ),
            _ => (None, "image default (pi / raspberry)".to_string()),
        }),
    }
}

async fn wifi(request: Option<&WifiRequest>) -> Option<WifiConfig> {
    let request = request?;
    let country = match &request.country {
        Some(country) => country.to_uppercase(),
        None => flasher_ops::wifi::list_wifi_networks()
            .await
            .ok()
            .and_then(|w| w.country)
            .unwrap_or_else(|| "US".to_string()),
    };
    Some(WifiConfig {
        ssid: request.ssid.trim().to_string(),
        password: request.password.clone(),
        country,
    })
}

pub fn recovery_text(
    hostname: Option<&str>,
    tang: &[String],
    cloud: bool,
    threshold: u32,
    recovery_key: &str,
) -> String {
    let servers: Vec<String> = tang
        .iter()
        .map(|url| format!("  - {}", url.trim()))
        .chain(cloud.then(|| "  - Aircast Cloud (this card's own key set)".to_string()))
        .collect();
    [
        "Aircast disk-encryption recovery key".to_string(),
        "KEEP THIS SECRET. It unlocks the device's data partition by hand if the".to_string(),
        "key servers are unreachable. It is NOT stored on the card.".to_string(),
        String::new(),
        format!("Device:    {}", hostname.unwrap_or("(unnamed device)")),
        format!("Unlock:    {threshold} of {} key server(s):", servers.len()),
        servers.join("\n"),
        String::new(),
        format!("Recovery key: {recovery_key}"),
        String::new(),
    ]
    .join("\n")
}

async fn encryption(
    request: Option<&EncryptionRequest>,
    hostname: Option<&str>,
) -> Result<Option<EncryptionConfig>, String> {
    let Some(request) = request else {
        return Ok(None);
    };
    let tang =
        futures_util::future::try_join_all(request.tang_servers.iter().map(|url| async move {
            flasher_ops::tang::fetch_tang_adv(url.clone())
                .await
                .map(|adv| TangServer {
                    url: url.trim().to_string(),
                    adv,
                })
        }))
        .await?;
    let threshold = request.threshold.unwrap_or(1);
    let recovery_key = flasher_ops::random_hex(24);
    let config = EncryptionConfig {
        tang_servers: tang,
        threshold,
        recovery_key: recovery_key.clone(),
    };
    let config = match request.cloud_unlock {
        true => {
            let label = flasher_ops::cloud::card_label(hostname.unwrap_or(""));
            let keyset = session::forget_if_signed_out(
                flasher_ops::cloud::create_keyset(&session::token(), &label).await,
            )?;
            flasher_ops::cloud::with_cloud_server(config, &keyset)
        }
        false => config,
    };
    save_recovery(
        &request.recovery_file,
        &recovery_text(
            hostname,
            &request.tang_servers,
            request.cloud_unlock,
            threshold,
            &recovery_key,
        ),
    )?;
    Ok(Some(config))
}

fn save_recovery(path: &str, text: &str) -> Result<(), String> {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(text.as_bytes()))
        .map_err(|e| format!("Could not save the recovery key to {path}: {e}"))
}

pub struct Prepared {
    pub card: BlockDevice,
    pub image: String,
}

pub async fn run(
    request: &FlashRequest,
    prepared: Prepared,
    cancel: Arc<AtomicBool>,
    events: FlashEvents,
) -> Result<FlashOutcome, String> {
    let hostname = request
        .hostname
        .as_deref()
        .map(str::trim)
        .filter(|h| !h.is_empty());
    let (access, ssh_summary) = access(request.ssh.as_ref())?;
    let wifi = wifi(request.wifi.as_ref()).await;
    let tailscale = request.tailscale.as_ref().map(|t| TailscaleConfig {
        control_server: t.control_server.clone().unwrap_or_default(),
        auth_key: t.auth_key.trim().to_string(),
    });
    let encryption = encryption(request.encryption.as_ref(), hostname).await?;

    let config = ProvisionConfig {
        hostname: hostname.map(str::to_string),
        wifi,
        tailscale,
        access,
        encryption,
        fight_mode: request.fight_mode,
        init_format: InitFormat::CloudInit,
    };

    let mut trace = FlashTrace::default();
    flasher_ops::flash::flash_image(
        &config,
        &prepared.image,
        &prepared.card.path,
        cancel,
        events,
        &mut trace,
    )
    .await?;

    Ok(FlashOutcome {
        disk: prepared.card.path,
        image: prepared.image,
        hostname: hostname.map(str::to_string),
        address: hostname.map(|h| format!("http://{h}.local")),
        ssh: ssh_summary,
        recovery_file: request.encryption.as_ref().map(|e| e.recovery_file.clone()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> FlashRequest {
        FlashRequest {
            disk: "/dev/disk4".into(),
            hostname: Some("falcon-01".into()),
            ..FlashRequest::default()
        }
    }

    #[test]
    fn a_plain_request_has_no_problems() {
        assert!(problems(&request(), 1).is_empty());
    }

    #[test]
    fn names_every_problem_at_once() {
        let bad = FlashRequest {
            disk: String::new(),
            hostname: Some("Falcon_01".into()),
            wifi: Some(WifiRequest {
                ssid: "home".into(),
                password: "short".into(),
                country: None,
            }),
            fight_mode: true,
            ..FlashRequest::default()
        };
        let found = problems(&bad, 0);
        assert_eq!(found.len(), 5, "{found:?}");
        assert!(found.iter().any(|p| p.contains("needs an SSH key")));
        assert!(found.iter().any(|p| p.contains("card")));
        assert!(found.iter().any(|p| p == HOSTNAME_RULE));
        assert!(found.iter().any(|p| p.contains("8–63")));
        assert!(found.iter().any(|p| p.contains("Fight mode needs")));
    }

    #[test]
    fn fight_mode_uses_the_only_key_or_asks_for_one() {
        let fight = FlashRequest {
            fight_mode: true,
            tailscale: Some(TailnetRequest {
                auth_key: "tskey-auth-x".into(),
                control_server: None,
            }),
            ..request()
        };
        assert!(problems(&fight, 1).is_empty());
        assert!(problems(&fight, 0)
            .iter()
            .any(|p| p.contains("needs an SSH key")));
        assert!(problems(&fight, 2)
            .iter()
            .any(|p| p.contains("needs an SSH key")));
    }

    #[test]
    fn card_flags_need_a_new_enough_image() {
        assert!(version_at_least("v0.3.6", CARD_FLAGS_MIN_IMAGE));
        assert!(version_at_least("v0.3.6-beta.1", CARD_FLAGS_MIN_IMAGE));
        assert!(version_at_least("0.4.0", CARD_FLAGS_MIN_IMAGE));
        assert!(!version_at_least("v0.3.5", CARD_FLAGS_MIN_IMAGE));
        assert!(!version_at_least("nonsense", CARD_FLAGS_MIN_IMAGE));
    }

    #[test]
    fn hostnames_follow_the_wizard_rule() {
        assert!(hostname_valid("falcon-01"));
        assert!(!hostname_valid("Falcon-01"));
        assert!(!hostname_valid("-falcon"));
        assert!(!hostname_valid("falcon-"));
        assert!(!hostname_valid(""));
        assert!(!hostname_valid(&"a".repeat(64)));
    }

    #[test]
    fn encryption_needs_a_server_and_a_fresh_recovery_file() {
        let dir = std::env::temp_dir().join(format!("flasher-cli-recovery-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let taken = dir.join("taken.txt");
        std::fs::write(&taken, "old").unwrap();
        let encrypted = FlashRequest {
            encryption: Some(EncryptionRequest {
                tang_servers: vec![],
                threshold: None,
                cloud_unlock: false,
                recovery_file: taken.to_string_lossy().to_string(),
            }),
            ..request()
        };
        let found = problems(&encrypted, 1);
        assert!(
            found.iter().any(|p| p.contains("Tang key server")),
            "{found:?}"
        );
        assert!(
            found.iter().any(|p| p.contains("already exists")),
            "{found:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_recovery_file_lists_every_server_it_needs() {
        let text = recovery_text(
            Some("falcon-01"),
            &["http://10.0.0.5:7500".to_string()],
            true,
            2,
            "abc123",
        );
        assert!(text.contains("Device:    falcon-01"));
        assert!(text.contains("Unlock:    2 of 2 key server(s):"));
        assert!(text.contains("  - http://10.0.0.5:7500"));
        assert!(text.contains("  - Aircast Cloud"));
        assert!(text.contains("Recovery key: abc123"));
    }
}
