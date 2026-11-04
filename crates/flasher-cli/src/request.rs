use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use flasher_ops::flash::{FlashEvents, FlashTrace};
use flasher_ops::{
    AccessConfig, BlockDevice, DownloadProgress, EncryptionConfig, InitFormat, ProvisionConfig,
    Release, SshMode, TailscaleConfig, TangServer, WifiConfig, MIN_FIGHT_PASSWORD,
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
    #[schemars(
        description = "Board id from list_boards, e.g. orangepi5. Defaults to rpi. Boards without built-in WiFi (see list_boards) need wifi left out unless a USB WiFi adapter is plugged in."
    )]
    pub board: Option<String>,
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
    #[schemars(
        description = "Put a saved backup (an id from list_backups) on the card; the drone restores it on first boot and restarts once. The fight mode, SSH password login, disk encryption and control server the backup records apply unless set here; for the same drone, a backup holding its tailnet node rejoins without a key."
    )]
    pub restore: Option<RestoreRequest>,
    #[serde(skip)]
    #[schemars(skip)]
    pub rejoins_tailnet: bool,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RestoreRequest {
    #[schemars(description = "Backup id, as list_backups reports it.")]
    pub backup: String,
    #[schemars(
        description = "true when the card goes back into the drone the backup came from (it returns as the same aircraft, and the hostname defaults to the backup's); false to copy the setup onto a new drone with its own identity."
    )]
    pub same_drone: bool,
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
        description = "Password for the pi account, at least 8 characters (12 in fight mode). Mode password only."
    )]
    pub password: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct TailnetRequest {
    #[schemars(
        description = "Pre-auth key (tskey-auth-… or hskey-auth-…), or an API key (tskey-api-… for Tailscale, hskey-api-… for Headscale) to mint a fresh one-time key for this card."
    )]
    pub auth_key: String,
    #[schemars(description = "Headscale control server URL. Leave out for Tailscale.")]
    pub control_server: Option<String>,
    #[serde(default)]
    #[schemars(
        description = "Headscale user the drone joins as. Needed with a Headscale API key."
    )]
    pub user: Option<String>,
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
    pub from_backup: Vec<String>,
    #[schemars(
        description = "Which aircastd the card runs: the newest from the image's apt channel (stable, or testing for a beta image), installed on first boot if it's newer than the image's, or the version built into the image."
    )]
    pub aircastd: String,
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
    let min_password = if request.fight_mode {
        MIN_FIGHT_PASSWORD
    } else {
        8
    };
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
            .filter(|s| s.password.as_deref().unwrap_or("").trim().chars().count() < min_password)
            .map(|_| format!("Make the device password at least {min_password} characters")),
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
        tailnet
            .filter(|t| flasher_ops::authkey::is_headscale_api_key(&t.auth_key))
            .filter(|t| t.control_server.is_none())
            .map(|_| "A Headscale API key needs the Headscale control server URL".to_string()),
        tailnet
            .filter(|t| flasher_ops::authkey::is_headscale_api_key(&t.auth_key))
            .filter(|t| t.user.as_deref().unwrap_or("").trim().is_empty())
            .map(|_| "A Headscale API key needs the user the drone joins as".to_string()),
        (request.fight_mode && tailnet.is_none() && !request.rejoins_tailnet).then(|| {
            "Fight mode needs a tailnet pre-auth key — the tailnet is the only way in".to_string()
        }),
        (request.fight_mode && request.ssh.is_none() && detected_keys != 1).then(|| {
            "Fight mode needs an SSH key or a device password — pass one, or turn SSH off; the image's default login would stay on"
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
        request
            .restore
            .as_ref()
            .and_then(|r| restore_file(r).err()),
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

pub fn wifi_note(request: &FlashRequest) -> Option<String> {
    let board = flasher_ops::releases::board(request.board.as_deref()).ok()?;
    let wifi = request.wifi.as_ref()?;
    (!board.builtin_wifi).then(|| {
        format!(
            "{} has no built-in WiFi — it joins {} only with a USB WiFi adapter plugged in",
            board.name, wifi.ssid
        )
    })
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
    board: Option<String>,
) -> Result<Release, String> {
    let releases = flasher_ops::releases::list_releases(channel, board)
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

async fn tailscale(request: Option<&TailnetRequest>) -> Result<Option<TailscaleConfig>, String> {
    let Some(request) = request else {
        return Ok(None);
    };
    let control_server = request.control_server.clone().unwrap_or_default();
    let auth_key = flasher_ops::authkey::resolve(
        &control_server,
        &request.auth_key,
        request.user.as_deref().unwrap_or(""),
    )
    .await?;
    Ok(Some(TailscaleConfig {
        control_server,
        auth_key,
    }))
}

fn restore_file(restore: &RestoreRequest) -> Result<String, String> {
    flasher_ops::paths::backup_dir().and_then(|dir| {
        flasher_ops::backup::restore_file(&dir, &restore.backup, restore.same_drone)
    })
}

fn backup_of(restore: &RestoreRequest) -> Option<flasher_ops::backup::Backup> {
    let dir = flasher_ops::paths::backup_dir().ok()?;
    flasher_ops::backup::get(&dir, &restore.backup).ok()
}

pub fn apply_backup(request: FlashRequest) -> (FlashRequest, Vec<String>) {
    match request.restore.as_ref().and_then(backup_of) {
        Some(flasher_ops::backup::Backup {
            card: Some(card),
            hostname,
            ..
        }) => with_card(request, &card, &hostname),
        Some(_) => (
            request,
            vec!["nothing — it doesn't record fight mode, encryption or remote access; set them yourself".to_string()],
        ),
        None => (request, Vec::new()),
    }
}

fn describe_encryption(e: &flasher_ops::backup::CardEncryption) -> String {
    let servers: Vec<String> = e
        .tang_urls
        .iter()
        .cloned()
        .chain(e.cloud.then(|| "Aircast Cloud".to_string()))
        .collect();
    match e.servers_known() {
        true => format!(
            "disk encryption, {} of {} key servers: {}",
            e.threshold,
            servers.len(),
            servers.join(", ")
        ),
        false => {
            "disk encryption — its key servers weren't recorded; add a Tang server or Cloud unlock"
                .to_string()
        }
    }
}

fn with_card(
    request: FlashRequest,
    card: &flasher_ops::backup::Card,
    host: &str,
) -> (FlashRequest, Vec<String>) {
    let rejoins = card.tailnet_node && request.restore.as_ref().is_some_and(|r| r.same_drone);
    let fight = card.fight_mode && !request.fight_mode;
    let password = card.password_login && request.ssh.is_none();
    let encryption = card.encryption.as_ref().filter(|_| {
        request
            .encryption
            .as_ref()
            .is_none_or(|e| e.tang_servers.is_empty() && !e.cloud_unlock)
    });
    let control_server = Some(card.control_server.clone())
        .filter(|server| !rejoins && !server.is_empty())
        .filter(|_| {
            request
                .tailscale
                .as_ref()
                .is_some_and(|t| t.control_server.is_none())
        });

    let taken = [
        rejoins.then(|| {
            format!("remote access — {host} rejoins the tailnet as the same node, no key written")
        }),
        fight.then(|| "fight mode".to_string()),
        password.then(|| {
            "SSH password login — set a device password, an SSH key or SSH off".to_string()
        }),
        encryption.map(describe_encryption),
        control_server
            .as_ref()
            .map(|server| format!("control server {server}")),
    ]
    .into_iter()
    .flatten()
    .collect();

    let resolved = FlashRequest {
        fight_mode: request.fight_mode || card.fight_mode,
        ssh: match password {
            true => Some(SshRequest {
                mode: SshChoice::Password,
                key: None,
                password: None,
            }),
            false => request.ssh,
        },
        tailscale: match rejoins {
            true => None,
            false => request.tailscale.map(|t| TailnetRequest {
                control_server: t.control_server.or(control_server),
                ..t
            }),
        },
        encryption: match encryption {
            Some(e) => Some(EncryptionRequest {
                tang_servers: e.tang_urls.clone(),
                threshold: request
                    .encryption
                    .as_ref()
                    .and_then(|r| r.threshold)
                    .or(e.servers_known().then_some(e.threshold)),
                cloud_unlock: e.cloud,
                recovery_file: request
                    .encryption
                    .map(|r| r.recovery_file)
                    .unwrap_or_default(),
            }),
            None => request.encryption,
        },
        rejoins_tailnet: rejoins,
        ..request
    };
    (resolved, taken)
}

fn restored_hostname(restore: Option<&RestoreRequest>) -> Option<String> {
    let restore = restore.filter(|r| r.same_drone)?;
    let dir = flasher_ops::paths::backup_dir().ok()?;
    flasher_ops::backup::get(&dir, &restore.backup)
        .ok()
        .map(|backup| backup.hostname)
        .filter(|name| hostname_valid(name))
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
        .filter(|h| !h.is_empty())
        .map(str::to_string)
        .or_else(|| restored_hostname(request.restore.as_ref()));
    let hostname = hostname.as_deref();
    let restore = request.restore.as_ref().map(restore_file).transpose()?;
    let (access, ssh_summary) = access(request.ssh.as_ref())?;
    let wifi = wifi(request.wifi.as_ref()).await;
    let tailscale = tailscale(request.tailscale.as_ref()).await?;
    let encryption = encryption(request.encryption.as_ref(), hostname).await?;

    let config = ProvisionConfig {
        hostname: hostname.map(str::to_string),
        wifi,
        tailscale,
        access,
        encryption,
        fight_mode: request.fight_mode,
        restore,
        cloud_enroll: None,
        aircastd: None,
        init_format: InitFormat::CloudInit,
    };

    let mut trace = FlashTrace::default();
    let aircastd = flasher_ops::flash::flash_image(
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
        from_backup: Vec::new(),
        aircastd: aircastd.to_string(),
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
    fn wifi_on_a_board_without_it_gets_a_note_naming_the_adapter() {
        let wifi = Some(WifiRequest {
            ssid: "field-net".into(),
            password: String::new(),
            country: None,
        });
        let on = |board: &str, wifi: Option<WifiRequest>| FlashRequest {
            board: Some(board.into()),
            wifi,
            ..request()
        };
        let note = wifi_note(&on("orangepi5", wifi.clone())).unwrap();
        assert!(
            note.contains("Orange Pi 5") && note.contains("USB WiFi adapter"),
            "{note}"
        );
        assert_eq!(wifi_note(&on("rpi", wifi)), None);
        assert_eq!(wifi_note(&on("rock-5b", None)), None);
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
                user: None,
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
    fn a_headscale_api_key_needs_the_server_and_a_user() {
        let with = |control_server: Option<&str>, user: Option<&str>| FlashRequest {
            tailscale: Some(TailnetRequest {
                auth_key: "hskey-api-abc-def".into(),
                control_server: control_server.map(String::from),
                user: user.map(String::from),
            }),
            ..request()
        };
        assert!(problems(&with(Some("https://hs.example.com"), Some("pilots")), 1).is_empty());
        assert_eq!(
            problems(&with(None, None), 1),
            [
                "A Headscale API key needs the Headscale control server URL",
                "A Headscale API key needs the user the drone joins as",
            ]
        );
    }

    #[test]
    fn fight_mode_takes_a_password_of_twelve_characters() {
        let fight = |password: &str| FlashRequest {
            fight_mode: true,
            tailscale: Some(TailnetRequest {
                auth_key: "tskey-auth-x".into(),
                control_server: None,
                user: None,
            }),
            ssh: Some(SshRequest {
                mode: SshChoice::Password,
                key: None,
                password: Some(password.into()),
            }),
            ..request()
        };
        assert!(problems(&fight("twelve-chars"), 0).is_empty());
        assert_eq!(
            problems(&fight("eleven-char"), 0),
            ["Make the device password at least 12 characters"]
        );
    }

    #[test]
    fn a_backup_that_is_not_on_this_computer_is_a_problem() {
        let restoring = FlashRequest {
            restore: Some(RestoreRequest {
                backup: "no-such-backup-0".into(),
                same_drone: true,
            }),
            ..request()
        };
        assert_eq!(
            problems(&restoring, 1),
            ["No backup named no-such-backup-0 on this computer"]
        );
        assert_eq!(restored_hostname(restoring.restore.as_ref()), None);
    }

    fn card() -> flasher_ops::backup::Card {
        flasher_ops::backup::Card {
            fight_mode: true,
            password_login: true,
            encryption: Some(flasher_ops::backup::CardEncryption {
                threshold: 2,
                tang_urls: vec!["https://tang.example.com".into()],
                cloud: true,
            }),
            control_server: "https://hs.example.com".into(),
            tailnet_node: true,
        }
    }

    fn restoring(same_drone: bool) -> FlashRequest {
        FlashRequest {
            restore: Some(RestoreRequest {
                backup: "falcon-20261101120000".into(),
                same_drone,
            }),
            ..request()
        }
    }

    #[test]
    fn the_backup_fills_in_what_was_not_asked_for() {
        let (new_drone, taken) = with_card(
            FlashRequest {
                tailscale: Some(TailnetRequest {
                    auth_key: "tskey-auth-x".into(),
                    control_server: None,
                    user: None,
                }),
                ..restoring(false)
            },
            &card(),
            "falcon",
        );
        assert!(new_drone.fight_mode);
        assert_eq!(new_drone.ssh.as_ref().unwrap().mode, SshChoice::Password);
        let encryption = new_drone.encryption.as_ref().unwrap();
        assert_eq!(encryption.tang_servers, ["https://tang.example.com"]);
        assert!(encryption.cloud_unlock);
        assert_eq!(encryption.threshold, Some(2));
        assert_eq!(
            new_drone
                .tailscale
                .as_ref()
                .unwrap()
                .control_server
                .as_deref(),
            Some("https://hs.example.com")
        );
        assert!(!new_drone.rejoins_tailnet);
        assert_eq!(taken.len(), 4, "{taken:?}");
        assert!(taken.contains(
            &"disk encryption, 2 of 2 key servers: https://tang.example.com, Aircast Cloud"
                .to_string()
        ));
    }

    #[test]
    fn an_explicit_flag_beats_the_backup() {
        let explicit = FlashRequest {
            ssh: Some(SshRequest {
                mode: SshChoice::Key,
                key: Some("ssh-ed25519 AAAA me@host".into()),
                password: None,
            }),
            encryption: Some(EncryptionRequest {
                tang_servers: vec!["http://10.0.0.5:7500".into()],
                threshold: None,
                cloud_unlock: false,
                recovery_file: "/tmp/key.txt".into(),
            }),
            tailscale: Some(TailnetRequest {
                auth_key: "hskey-auth-x".into(),
                control_server: Some("https://mine.example.com".into()),
                user: None,
            }),
            fight_mode: true,
            ..restoring(false)
        };
        let (resolved, taken) = with_card(explicit.clone(), &card(), "falcon");
        assert_eq!(resolved.ssh.unwrap().mode, SshChoice::Key);
        let encryption = resolved.encryption.unwrap();
        assert_eq!(encryption.tang_servers, ["http://10.0.0.5:7500"]);
        assert!(!encryption.cloud_unlock);
        assert_eq!(
            resolved.tailscale.unwrap().control_server.as_deref(),
            Some("https://mine.example.com")
        );
        assert!(taken.is_empty(), "{taken:?}");
    }

    #[test]
    fn an_explicit_threshold_or_recovery_file_keeps_the_backup_servers() {
        let (resolved, _) = with_card(
            FlashRequest {
                encryption: Some(EncryptionRequest {
                    tang_servers: vec![],
                    threshold: Some(1),
                    cloud_unlock: false,
                    recovery_file: "/tmp/key.txt".into(),
                }),
                ..restoring(false)
            },
            &card(),
            "falcon",
        );
        let encryption = resolved.encryption.unwrap();
        assert_eq!(encryption.tang_servers, ["https://tang.example.com"]);
        assert_eq!(encryption.threshold, Some(1));
        assert_eq!(encryption.recovery_file, "/tmp/key.txt");
    }

    #[test]
    fn the_same_drone_rejoins_its_tailnet_without_a_key() {
        let with_key = FlashRequest {
            tailscale: Some(TailnetRequest {
                auth_key: "tskey-auth-x".into(),
                control_server: None,
                user: None,
            }),
            ssh: Some(SshRequest {
                mode: SshChoice::Off,
                key: None,
                password: None,
            }),
            ..restoring(true)
        };
        let (same, taken) = with_card(with_key, &card(), "falcon");
        assert!(same.tailscale.is_none());
        assert!(same.rejoins_tailnet);
        assert!(taken[0].contains("falcon rejoins the tailnet"), "{taken:?}");
        let fight_problems = problems(
            &FlashRequest {
                encryption: None,
                restore: None,
                ..same
            },
            0,
        );
        assert!(fight_problems.is_empty(), "{fight_problems:?}");

        let (new, _) = with_card(
            FlashRequest {
                ssh: Some(SshRequest {
                    mode: SshChoice::Off,
                    key: None,
                    password: None,
                }),
                ..restoring(false)
            },
            &card(),
            "falcon",
        );
        assert!(!new.rejoins_tailnet);
        assert!(problems(
            &FlashRequest {
                encryption: None,
                restore: None,
                ..new
            },
            0
        )
        .iter()
        .any(|p| p.contains("Fight mode needs a tailnet")));
    }

    #[test]
    fn unknown_key_servers_turn_encryption_on_and_ask_for_them() {
        let unknown = flasher_ops::backup::Card {
            encryption: Some(flasher_ops::backup::CardEncryption::default()),
            ..flasher_ops::backup::Card::default()
        };
        let (resolved, taken) = with_card(restoring(false), &unknown, "falcon");
        let encryption = resolved.encryption.as_ref().unwrap();
        assert!(encryption.tang_servers.is_empty() && !encryption.cloud_unlock);
        assert_eq!(encryption.threshold, None);
        assert!(taken[0].contains("weren't recorded"), "{taken:?}");
        assert!(problems(
            &FlashRequest {
                restore: None,
                ..resolved
            },
            1
        )
        .iter()
        .any(|p| p.contains("Tang key server")));
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
