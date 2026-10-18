use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WifiConfig {
    pub ssid: String,
    pub password: String,
    pub country: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum InitFormat {
    CloudInit,
    FirstRun,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleConfig {
    pub control_server: String,
    pub auth_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TangServer {
    pub url: String,
    pub adv: String,
}

impl TangServer {
    fn is_complete(&self) -> bool {
        !self.url.trim().is_empty() && !self.adv.trim().is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncryptionConfig {
    pub tang_servers: Vec<TangServer>,
    pub threshold: u32,
    pub recovery_key: String,
}

impl EncryptionConfig {
    pub fn is_complete(&self) -> bool {
        let servers = self.tang_servers.len() as u32;
        !self.recovery_key.trim().is_empty()
            && servers >= 1
            && self.tang_servers.iter().all(TangServer::is_complete)
            && self.threshold >= 1
            && self.threshold <= servers
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum SshMode {
    KeyOnly,
    Password,
    Disabled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessConfig {
    pub ssh: SshMode,
    #[serde(default)]
    pub authorized_key: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

impl AccessConfig {
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvisionConfig {
    pub hostname: Option<String>,
    pub wifi: Option<WifiConfig>,
    pub tailscale: Option<TailscaleConfig>,
    #[serde(default)]
    pub access: Option<AccessConfig>,
    #[serde(default)]
    pub encryption: Option<EncryptionConfig>,
    #[serde(default)]
    pub fight_mode: bool,
    pub init_format: InitFormat,
}

impl ProvisionConfig {
    pub fn is_empty(&self) -> bool {
        self.hostname.is_none()
            && self.wifi.is_none()
            && self.tailscale.is_none()
            && !self.access.as_ref().is_some_and(AccessConfig::is_effective)
    }
}
