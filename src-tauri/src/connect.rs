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

#[tauri::command]
pub async fn probe_device(url: String) -> bool {
    probe_client()
        .get(format!("{url}/healthz"))
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

#[tauri::command]
pub async fn join_wifi(ssid: String, password: String) -> Result<(), String> {
    join_wifi_impl(&ssid, &password).await
}

#[cfg(target_os = "macos")]
async fn join_wifi_impl(ssid: &str, password: &str) -> Result<(), String> {
    let dev = crate::flasher::wifi_device_macos().await;
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
