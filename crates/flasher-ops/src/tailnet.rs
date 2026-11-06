use std::net::{IpAddr, Ipv4Addr};
use std::sync::OnceLock;
use std::time::Duration;

fn probe_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .expect("default reqwest client")
    })
}

fn health_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .local_address(IpAddr::V4(Ipv4Addr::UNSPECIFIED))
            .build()
            .expect("health probe client")
    })
}

const MDNS_GROUP: (Ipv4Addr, u16) = (Ipv4Addr::new(224, 0, 0, 251), 5353);
const MDNS_WAIT: Duration = Duration::from_millis(1500);

fn mdns_query(host: &str) -> Option<Vec<u8>> {
    let mut packet = simple_dns::Packet::new_query(0);
    packet.questions.push(simple_dns::Question::new(
        simple_dns::Name::new(host).ok()?,
        simple_dns::TYPE::A.into(),
        simple_dns::CLASS::IN.into(),
        false,
    ));
    packet.build_bytes_vec().ok()
}

fn a_records(reply: &[u8], host: &str) -> Vec<Ipv4Addr> {
    simple_dns::Packet::parse(reply)
        .map(|packet| {
            packet
                .answers
                .iter()
                .chain(packet.additional_records.iter())
                .filter(|record| record.name.to_string().eq_ignore_ascii_case(host))
                .filter_map(|record| match &record.rdata {
                    simple_dns::rdata::RData::A(a) => Some(Ipv4Addr::from(a.address)),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

fn query_socket(local: Ipv4Addr) -> std::io::Result<tokio::net::UdpSocket> {
    let socket = socket2::Socket::new(socket2::Domain::IPV4, socket2::Type::DGRAM, None)?;
    socket.set_multicast_if_v4(&local)?;
    socket.bind(&std::net::SocketAddr::from((local, 0)).into())?;
    socket.set_nonblocking(true)?;
    tokio::net::UdpSocket::from_std(socket.into())
}

async fn ask_on(local: Ipv4Addr, query: &[u8], host: &str) -> Vec<Ipv4Addr> {
    let Ok(socket) = query_socket(local) else {
        return Vec::new();
    };
    if socket.send_to(query, MDNS_GROUP).await.is_err() {
        return Vec::new();
    }
    let mut reply = [0u8; 1500];
    match tokio::time::timeout(MDNS_WAIT, socket.recv(&mut reply)).await {
        Ok(Ok(len)) => a_records(&reply[..len], host),
        _ => Vec::new(),
    }
}

async fn resolve_local(host: &str) -> Vec<Ipv4Addr> {
    let Some(query) = mdns_query(host) else {
        return Vec::new();
    };
    let locals: Vec<Ipv4Addr> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|interface| match interface.ip() {
            IpAddr::V4(ip) if !ip.is_loopback() && !ip.is_link_local() => Some(ip),
            _ => None,
        })
        .collect();
    futures::future::join_all(locals.into_iter().map(|local| ask_on(local, &query, host)))
        .await
        .into_iter()
        .flatten()
        .collect()
}

async fn candidates(url: &str) -> Vec<String> {
    let parsed = url::Url::parse(url).ok();
    let host = parsed
        .as_ref()
        .and_then(|u| u.host_str())
        .filter(|host| host.ends_with(".local"))
        .map(str::to_string);
    let resolved = match (parsed, host) {
        (Some(parsed), Some(host)) => resolve_local(&host)
            .await
            .into_iter()
            .filter_map(|ip| {
                let mut by_ip = parsed.clone();
                by_ip.set_ip_host(IpAddr::V4(ip)).ok()?;
                Some(by_ip.as_str().trim_end_matches('/').to_string())
            })
            .collect(),
        _ => Vec::new(),
    };
    [resolved, vec![url.to_string()]].concat()
}

async fn aircastd_ip(url: &str, timeout: Duration) -> Option<IpAddr> {
    let probes = candidates(url)
        .await
        .into_iter()
        .map(|candidate| Box::pin(async move { healthz_ip(&candidate, timeout).await.ok_or(()) }));
    futures::future::select_ok(probes)
        .await
        .ok()
        .map(|(ip, _)| ip)
}

async fn healthz_ip(url: &str, timeout: Duration) -> Option<IpAddr> {
    let response = health_client()
        .get(format!("{url}/healthz"))
        .timeout(timeout)
        .send()
        .await
        .ok()?;
    let ip = response.remote_addr()?.ip();
    if response.status() != reqwest::StatusCode::OK {
        return None;
    }
    (response.text().await.ok()?.trim() == "ok").then_some(ip)
}

const LAN_PROBE_TIMEOUT: Duration = Duration::from_secs(8);
pub async fn probe_device(url: String) -> Option<String> {
    aircastd_ip(&url, LAN_PROBE_TIMEOUT)
        .await
        .map(|ip| ip.to_string())
}

const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

fn login_error(e: &reqwest::Error) -> String {
    if e.is_timeout() || e.is_connect() {
        "Couldn't reach the device — it may still be booting. Try again in a minute.".to_string()
    } else {
        "The device couldn't start Tailscale sign-in. Open the device page to set it up there."
            .to_string()
    }
}
pub async fn tailscale_login(url: String) -> Result<String, String> {
    #[derive(serde::Deserialize)]
    struct ConnectResponse {
        #[serde(rename = "authURL")]
        auth_url: Option<String>,
    }

    let response = probe_client()
        .post(format!("{url}/api/tailscale/connection"))
        .timeout(LOGIN_TIMEOUT)
        .send()
        .await
        .map_err(|e| login_error(&e))?
        .error_for_status()
        .map_err(|e| login_error(&e))?;

    response
        .json::<ConnectResponse>()
        .await
        .map(|r| r.auth_url.unwrap_or_default())
        .map_err(|e| login_error(&e))
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleState {
    pub connected: bool,
    pub name: String,
}
pub async fn tailscale_status(url: String) -> Result<TailscaleState, String> {
    #[derive(serde::Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct StatusResponse {
        #[serde(default)]
        logged_in: bool,
        #[serde(default)]
        tailnet: String,
        #[serde(default)]
        dns_name: String,
    }

    let status = probe_client()
        .get(format!("{url}/api/tailscale"))
        .send()
        .await
        .map_err(|e| login_error(&e))?
        .error_for_status()
        .map_err(|e| login_error(&e))?
        .json::<StatusResponse>()
        .await
        .map_err(|e| login_error(&e))?;

    let name = match status.dns_name.trim_end_matches('.') {
        "" => status.tailnet,
        dns => dns.to_string(),
    };

    Ok(TailscaleState {
        connected: status.logged_in,
        name,
    })
}
pub async fn set_device_control_server(url: String, control_server: String) -> Result<(), String> {
    probe_client()
        .put(format!("{url}/api/tailscale/control"))
        .json(&serde_json::json!({ "controlServer": control_server, "authKey": "" }))
        .timeout(LOGIN_TIMEOUT)
        .send()
        .await
        .map_err(|e| login_error(&e))?
        .error_for_status()
        .map_err(|e| login_error(&e))
        .map(|_| ())
}
const PEER_PROBE_TIMEOUT: Duration = Duration::from_secs(5);
pub async fn probe_devices(urls: Vec<String>) -> Vec<String> {
    let probes = urls.into_iter().map(|url| {
        tokio::spawn(async move { aircastd_ip(&url, PEER_PROBE_TIMEOUT).await.map(|_| url) })
    });

    let mut reachable = Vec::new();
    for probe in probes.collect::<Vec<_>>() {
        if let Ok(Some(url)) = probe.await {
            reachable.push(url);
        }
    }
    reachable
}

fn tailscale_cli() -> std::path::PathBuf {
    #[cfg(target_os = "macos")]
    let candidates: Vec<String> = [
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    #[cfg(target_os = "windows")]
    let candidates: Vec<String> = ["ProgramFiles", "ProgramFiles(x86)"]
        .iter()
        .filter_map(|var| std::env::var(var).ok())
        .map(|dir| format!("{dir}\\Tailscale\\tailscale.exe"))
        .collect();

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let candidates: Vec<String> = [
        "/usr/bin/tailscale",
        "/usr/local/bin/tailscale",
        "/snap/bin/tailscale",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    candidates
        .into_iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
        .unwrap_or_else(|| std::path::PathBuf::from("tailscale"))
}

#[cfg(target_os = "windows")]
fn cli_command(program: &std::path::Path) -> tokio::process::Command {
    crate::hidden_command(&program.to_string_lossy())
}

#[cfg(not(target_os = "windows"))]
fn cli_command(program: &std::path::Path) -> tokio::process::Command {
    let mut command = tokio::process::Command::new(program);
    command.env("TAILSCALE_BE_CLI", "1");
    command
}

const CLI_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalTailscale {
    pub installed: bool,
    pub running: bool,
    pub name: String,
    pub tailnet: String,
    pub magic_dns_suffix: String,
    pub control_url: String,
    pub self_hosted: bool,
    pub peers: Vec<TailnetPeer>,
}

async fn control_url(cli: &std::path::Path) -> String {
    #[derive(serde::Deserialize)]
    struct Prefs {
        #[serde(rename = "ControlURL", default)]
        control_url: String,
    }

    let output = tokio::time::timeout(
        CLI_TIMEOUT,
        cli_command(cli).args(["debug", "prefs"]).output(),
    )
    .await;

    let Ok(Ok(output)) = output else {
        return String::new();
    };
    serde_json::from_slice::<Prefs>(&output.stdout)
        .map(|p| p.control_url.trim_end_matches('/').to_string())
        .unwrap_or_default()
}

fn is_self_hosted(control_url: &str) -> bool {
    !control_url.is_empty() && !control_url.contains("tailscale.com")
}

#[derive(serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TailnetPeer {
    pub host_name: String,
    pub dns_name: String,
    pub os: String,
    pub online: bool,
    pub ip: String,
    pub same_tailnet: bool,
}
pub async fn local_tailscale() -> LocalTailscale {
    #[derive(serde::Deserialize)]
    struct CliStatus {
        #[serde(rename = "BackendState", default)]
        backend_state: String,
        #[serde(rename = "Self", default)]
        this: Option<CliSelf>,
        #[serde(rename = "CurrentTailnet", default)]
        tailnet: Option<CliTailnet>,
        #[serde(rename = "Peer", default)]
        peers: std::collections::HashMap<String, CliPeer>,
    }

    #[derive(serde::Deserialize)]
    struct CliSelf {
        #[serde(rename = "DNSName", default)]
        dns_name: String,
    }

    #[derive(serde::Deserialize)]
    struct CliPeer {
        #[serde(rename = "HostName", default)]
        host_name: String,
        #[serde(rename = "DNSName", default)]
        dns_name: String,
        #[serde(rename = "OS", default)]
        os: String,
        #[serde(rename = "Online", default)]
        online: bool,
        #[serde(rename = "TailscaleIPs", default)]
        ips: Vec<String>,
    }

    #[derive(serde::Deserialize)]
    struct CliTailnet {
        #[serde(rename = "Name", default)]
        name: String,
        #[serde(rename = "MagicDNSSuffix", default)]
        magic_dns_suffix: String,
    }

    let cli = tailscale_cli();
    let spawned = tokio::time::timeout(
        CLI_TIMEOUT,
        cli_command(&cli).args(["status", "--json"]).output(),
    )
    .await;

    let output = match spawned {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => return LocalTailscale::default(),
        Err(_) => {
            return LocalTailscale {
                installed: true,
                ..Default::default()
            }
        }
    };

    let installed = LocalTailscale {
        installed: true,
        ..Default::default()
    };

    let Ok(status) = serde_json::from_slice::<CliStatus>(&output.stdout) else {
        return installed;
    };

    let control = control_url(&cli).await;
    let tailnet = status.tailnet;
    let suffix = tailnet
        .as_ref()
        .map(|t| t.magic_dns_suffix.clone())
        .unwrap_or_default();

    let mut peers: Vec<TailnetPeer> = status
        .peers
        .into_values()
        .filter(|p| !p.dns_name.is_empty())
        .map(|p| {
            let dns_name = p.dns_name.trim_end_matches('.').to_string();
            TailnetPeer {
                same_tailnet: !suffix.is_empty() && dns_name.ends_with(&format!(".{suffix}")),
                host_name: p.host_name,
                dns_name,
                os: p.os,
                online: p.online,
                ip: p.ips.into_iter().next().unwrap_or_default(),
            }
        })
        .collect();

    peers.sort_by(|a, b| {
        b.online
            .cmp(&a.online)
            .then_with(|| a.dns_name.cmp(&b.dns_name))
    });

    LocalTailscale {
        self_hosted: is_self_hosted(&control),
        control_url: control,
        peers,
        installed: true,
        running: status.backend_state == "Running",
        name: status
            .this
            .map(|s| s.dns_name.trim_end_matches('.').to_string())
            .unwrap_or_default(),
        tailnet: tailnet.as_ref().map(|t| t.name.clone()).unwrap_or_default(),
        magic_dns_suffix: tailnet.map(|t| t.magic_dns_suffix).unwrap_or_default(),
    }
}
pub async fn join_wifi(ssid: String, password: String) -> Result<(), String> {
    join_wifi_impl(&ssid, &password).await
}

#[cfg(target_os = "macos")]
async fn join_wifi_impl(ssid: &str, password: &str) -> Result<(), String> {
    let dev = crate::wifi::wifi_device_macos().await;
    let out = tokio::process::Command::new("networksetup")
        .args(["-setairportnetwork", &dev, ssid, password])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    let msg = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match out.status.success() && msg.is_empty() {
        true => Ok(()),
        false => Err(if msg.is_empty() {
            "Could not join the network.".to_string()
        } else {
            msg
        }),
    }
}

#[cfg(target_os = "windows")]
async fn join_wifi_impl(ssid: &str, password: &str) -> Result<(), String> {
    let esc = |s: &str| {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let profile = format!(
        r#"<?xml version="1.0"?>
<WLANProfile xmlns="http://www.microsoft.com/networking/WLAN/profile/v1">
  <name>{ssid}</name>
  <SSIDConfig><SSID><name>{ssid}</name></SSID></SSIDConfig>
  <connectionType>ESS</connectionType>
  <connectionMode>manual</connectionMode>
  <MSM><security>
    <authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption>
    <sharedKey><keyType>passPhrase</keyType><protected>false</protected><keyMaterial>{password}</keyMaterial></sharedKey>
  </security></MSM>
</WLANProfile>"#,
        ssid = esc(ssid),
        password = esc(password),
    );
    let path = std::env::temp_dir().join("aircast-hotspot-profile.xml");
    tokio::fs::write(&path, profile)
        .await
        .map_err(|e| e.to_string())?;
    let add = crate::hidden_command("netsh")
        .args([
            "wlan",
            "add",
            "profile",
            &format!("filename={}", path.display()),
        ])
        .output()
        .await;
    let _ = tokio::fs::remove_file(&path).await;
    let add = add.map_err(|e| e.to_string())?;
    if !add.status.success() {
        return Err(String::from_utf8_lossy(&add.stdout).trim().to_string());
    }
    let connect = crate::hidden_command("netsh")
        .args(["wlan", "connect", &format!("name={ssid}")])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    match connect.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&connect.stdout).trim().to_string()),
    }
}

#[cfg(target_os = "linux")]
async fn join_wifi_impl(ssid: &str, password: &str) -> Result<(), String> {
    let out = tokio::process::Command::new("nmcli")
        .args(["device", "wifi", "connect", ssid, "password", password])
        .output()
        .await
        .map_err(|e| e.to_string())?;
    match out.status.success() {
        true => Ok(()),
        false => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn serve(reply: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        url
    }

    #[tokio::test]
    async fn aircastd_answers_with_the_ip_it_was_reached_on() {
        let url =
            serve("HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
        assert_eq!(probe_device(url).await.as_deref(), Some("127.0.0.1"));
    }

    fn reply(name: &str, ip: Ipv4Addr) -> Vec<u8> {
        let mut packet = simple_dns::Packet::new_reply(0);
        packet.answers.push(simple_dns::ResourceRecord::new(
            simple_dns::Name::new(name).unwrap(),
            simple_dns::CLASS::IN,
            10,
            simple_dns::rdata::RData::A(simple_dns::rdata::A { address: ip.into() }),
        ));
        packet.build_bytes_vec().unwrap()
    }

    #[test]
    fn an_mdns_reply_names_the_drone_s_ipv4() {
        let ip = Ipv4Addr::new(192, 168, 0, 117);
        assert_eq!(
            a_records(&reply("owel-red-test.local", ip), "owel-red-test.local"),
            vec![ip]
        );
        assert!(a_records(&reply("other.local", ip), "owel-red-test.local").is_empty());
        assert!(a_records(b"junk", "owel-red-test.local").is_empty());
    }

    #[tokio::test]
    async fn a_url_by_ip_is_probed_as_given() {
        assert_eq!(
            candidates("http://10.42.0.1").await,
            vec!["http://10.42.0.1"]
        );
    }

    #[tokio::test]
    async fn a_captive_portal_that_answers_every_name_is_not_a_device() {
        let redirect = serve("HTTP/1.1 302 Found\r\nLocation: http://10.0.0.1/\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
        let page = serve(
            "HTTP/1.1 200 OK\r\nContent-Length: 13\r\nConnection: close\r\n\r\n<html></html>",
        )
        .await;
        assert_eq!(probe_device(redirect).await, None);
        assert_eq!(probe_device(page.clone()).await, None);
        assert!(probe_devices(vec![page]).await.is_empty());
    }
}
