use schemars::JsonSchema;
use serde::Serialize;

const HOTSPOT_URL: &str = "http://10.42.0.1";

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub name: String,
    pub address: String,
    pub ip: String,
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct Fleet {
    pub tailnet: String,
    pub devices: Vec<Device>,
    pub offline: usize,
}

pub async fn devices() -> Result<Fleet, String> {
    let local = flasher_ops::tailnet::local_tailscale().await;
    if !local.installed {
        return Err(
            "Tailscale isn't installed on this computer, so there is no tailnet to search".into(),
        );
    }
    if !local.running {
        return Err(
            "Tailscale isn't signed in on this computer — sign in to see your devices".into(),
        );
    }
    let linux: Vec<_> = local
        .peers
        .into_iter()
        .filter(|peer| peer.os.eq_ignore_ascii_case("linux"))
        .collect();
    let offline = linux.iter().filter(|peer| !peer.online).count();
    let candidates: Vec<_> = linux.into_iter().filter(|peer| peer.online).collect();
    let reachable = flasher_ops::tailnet::probe_devices(
        candidates
            .iter()
            .map(|peer| format!("http://{}", peer.dns_name))
            .collect(),
    )
    .await;
    Ok(Fleet {
        tailnet: local.tailnet,
        offline,
        devices: candidates
            .into_iter()
            .map(|peer| Device {
                address: format!("http://{}", peer.dns_name),
                name: peer.host_name,
                ip: peer.ip,
            })
            .filter(|device| reachable.contains(&device.address))
            .collect(),
    })
}

pub fn address(target: &str) -> String {
    match target.starts_with("http://") || target.starts_with("https://") {
        true => target.trim_end_matches('/').to_string(),
        false => format!("http://{target}"),
    }
}

pub async fn find(hostname: &str) -> Option<String> {
    let candidates = [format!("http://{hostname}.local"), HOTSPOT_URL.to_string()];
    let reachable = flasher_ops::tailnet::probe_devices(candidates.to_vec()).await;
    candidates.into_iter().find(|url| reachable.contains(url))
}

#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DeviceTailnet {
    pub connected: bool,
    pub name: String,
}

pub async fn status(target: &str) -> Result<DeviceTailnet, String> {
    flasher_ops::tailnet::tailscale_status(address(target))
        .await
        .map(|state| DeviceTailnet {
            connected: state.connected,
            name: state.name,
        })
}

pub async fn login(target: &str, control_server: Option<&str>) -> Result<String, String> {
    let url = address(target);
    if let Some(server) = control_server {
        flasher_ops::tailnet::set_device_control_server(url.clone(), server.to_string()).await?;
    }
    flasher_ops::tailnet::tailscale_login(url).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_name_becomes_an_http_address() {
        assert_eq!(address("falcon-01.local"), "http://falcon-01.local");
        assert_eq!(address("http://10.42.0.1/"), "http://10.42.0.1");
        assert_eq!(address("https://falcon.ts.net"), "https://falcon.ts.net");
    }
}
