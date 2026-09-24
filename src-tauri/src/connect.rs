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

/// An `.local` name has to be resolved over mDNS before the first byte moves,
/// and a cold multicast lookup on a busy network can take several seconds on
/// its own — longer than the shared client's budget for a plain HTTP call.
const LAN_PROBE_TIMEOUT: Duration = Duration::from_secs(8);

#[tauri::command]
pub async fn probe_device(url: String) -> bool {
    probe_client()
        .get(format!("{url}/healthz"))
        .timeout(LAN_PROBE_TIMEOUT)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Reaching Tailscale's coordination server to mint a login URL is a round trip
/// over the device's own uplink, so it gets far longer than a `/healthz` probe.
const LOGIN_TIMEOUT: Duration = Duration::from_secs(20);

/// Operator-facing text for a failed call to aircastd. The transport error is
/// jargon the operator can do nothing with; what they need is which of the two
/// recoverable situations they are in.
fn login_error(e: &reqwest::Error) -> String {
    if e.is_timeout() || e.is_connect() {
        "Couldn't reach the device — it may still be booting. Try again in a minute.".to_string()
    } else {
        "The device couldn't start Tailscale sign-in. Open the device page to set it up there."
            .to_string()
    }
}

/// Starts aircastd's headless Tailscale login and returns the URL the operator
/// opens to sign in (creating a tailnet if they have none). Empty when the
/// device is already connected.
#[tauri::command]
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

/// What the device reports about its own tailnet membership. Polled while the
/// operator finishes signing in, so the screen can confirm the join instead of
/// leaving them to guess.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleState {
    pub connected: bool,
    pub name: String,
}

#[tauri::command]
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

    Ok(TailscaleState { connected: status.logged_in, name })
}

/// Points a device at a coordination server before it signs in. Without this,
/// a Headscale shop's freshly flashed device would try to sign in to
/// Tailscale's servers, where its operator has no account.
#[tauri::command]
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

/// Opens a device's own page in an app window rather than the system browser,
/// so managing a device never leaves the app.
///
/// A native window, not an iframe: the app's origin is a secure context, so an
/// embedded `http://` frame is mixed active content and the webviews disagree
/// about blocking it per platform. A top-level document has no such problem.
///
/// The device page gets no Tauri APIs — the capability in `capabilities/` is
/// scoped to the `main` window, so this webview inherits nothing.
#[tauri::command]
pub async fn open_device_window(
    app: tauri::AppHandle,
    url: String,
    title: String,
) -> Result<(), String> {
    use tauri::Manager;

    // Labels allow a restricted alphabet; a DNS name is not one.
    let label = format!(
        "device-{}",
        title
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
    );

    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        return existing.set_focus().map_err(|e| e.to_string());
    }

    let target = url.parse().map_err(|_| format!("Not a valid address: {url}"))?;

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(target))
        .title(title)
        .inner_size(1100.0, 760.0)
        .min_inner_size(700.0, 500.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

/// A tailnet round trip can need DERP setup on first contact, so peers get a
/// longer budget than a LAN `/healthz` probe.
const PEER_PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Which of these addresses are running aircastd. `/healthz` is the signature —
/// "is it Linux" is not: a tailnet's Linux peers include servers and stray Pis.
/// Probed in parallel, so the wall clock is one timeout, not N.
#[tauri::command]
pub async fn probe_devices(urls: Vec<String>) -> Vec<String> {
    let probes = urls.into_iter().map(|url| {
        tokio::spawn(async move {
            let ok = probe_client()
                .get(format!("{url}/healthz"))
                .timeout(PEER_PROBE_TIMEOUT)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            ok.then_some(url)
        })
    });

    let mut reachable = Vec::new();
    for probe in probes.collect::<Vec<_>>() {
        if let Ok(Some(url)) = probe.await {
            reachable.push(url);
        }
    }
    reachable
}

/// Where the Tailscale CLI usually lives. These are a fast path, not the whole
/// mechanism: `tailscale` is not on `PATH` after a Mac App Store install, so
/// the app bundle's own binary has to be found by path. Anything unusual —
/// snap, Nix, Windows on a drive that isn't C: — falls through to a bare
/// `PATH` lookup, and a spawn failure is what "not installed" means.
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
    crate::flasher::hidden_command(&program.to_string_lossy())
}

#[cfg(not(target_os = "windows"))]
fn cli_command(program: &std::path::Path) -> tokio::process::Command {
    tokio::process::Command::new(program)
}

/// A wedged `tailscaled` makes `tailscale status` hang; the screen must not.
const CLI_TIMEOUT: Duration = Duration::from_secs(3);

/// Whether *this computer* can reach the tailnet. The drone joining is only
/// half the job — without the client running here, the name the device reports
/// resolves to nothing.
#[derive(serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct LocalTailscale {
    pub installed: bool,
    pub running: bool,
    pub name: String,
    pub tailnet: String,
    pub magic_dns_suffix: String,
    /// The coordination server this computer is signed in to. Empty when it
    /// could not be read.
    pub control_url: String,
    /// True when `control_url` is a Headscale (or other) server rather than
    /// Tailscale's own. Keys, sign-in and the device's own control server all
    /// have to follow this, not the Tailscale defaults.
    pub self_hosted: bool,
    pub peers: Vec<TailnetPeer>,
}

/// `tailscale status --json` does not carry the control server; prefs do.
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

/// One machine already on the tailnet, as the local client sees it. Shared
/// devices from another tailnet come through with an empty `DNSName` and are
/// dropped — they cannot be addressed by name and cannot collide.
#[derive(serde::Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TailnetPeer {
    pub host_name: String,
    pub dns_name: String,
    pub os: String,
    pub online: bool,
    pub ip: String,
    /// False for a device shared in from a different tailnet. Only same-tailnet
    /// names can collide, since that is the scope Tailscale renames within.
    pub same_tailnet: bool,
}

#[tauri::command]
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

    // A spawn error is the only reliable "not installed" signal, since the
    // binary may have been found on PATH rather than at a known location.
    let output = match spawned {
        Ok(Ok(output)) => output,
        Ok(Err(_)) => return LocalTailscale::default(),
        Err(_) => return LocalTailscale { installed: true, ..Default::default() },
    };

    let installed = LocalTailscale { installed: true, ..Default::default() };

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

    // Online first, then by name — the list is read to find a machine, and an
    // offline one is not what the reader is looking for.
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

#[tauri::command]
pub async fn join_wifi(ssid: String, password: String) -> Result<(), String> {
    join_wifi_impl(&ssid, &password).await
}

#[cfg(target_os = "macos")]
async fn join_wifi_impl(ssid: &str, password: &str) -> Result<(), String> {
    let dev = crate::flasher::wifi::wifi_device_macos().await;
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
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
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
    let add = crate::flasher::hidden_command("netsh")
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
    let connect = crate::flasher::hidden_command("netsh")
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
