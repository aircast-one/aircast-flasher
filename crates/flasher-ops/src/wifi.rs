#[cfg(target_os = "windows")]
use super::hidden_command;
use super::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct WifiNetworks {
    pub current: Option<String>,
    pub known: Vec<String>,
    pub country: Option<String>,
}

fn region_from_locale(locale: &str) -> Option<String> {
    let base = locale.split('@').next().unwrap_or(locale);
    let parts: Vec<&str> = base.split(['_', '-']).filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return None;
    }
    let region = parts.last()?;
    if region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(region.to_uppercase())
    } else {
        None
    }
}
pub async fn list_wifi_networks() -> Result<WifiNetworks, String> {
    #[cfg(target_os = "macos")]
    {
        list_wifi_macos().await
    }
    #[cfg(target_os = "linux")]
    {
        list_wifi_linux().await
    }
    #[cfg(target_os = "windows")]
    {
        list_wifi_windows().await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Ok(WifiNetworks {
            current: None,
            known: vec![],
            country: None,
        })
    }
}

pub async fn test_wifi(ssid: &str, password: &str) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    {
        test_wifi_macos(ssid, password).await
    }
    #[cfg(target_os = "linux")]
    {
        test_wifi_linux(ssid, password).await
    }
    #[cfg(target_os = "windows")]
    {
        test_wifi_windows(ssid, password).await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (ssid, password);
        Err("Testing WiFi is not supported on this computer.".to_string())
    }
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
const TEST_PROFILE: &str = "aircast-flasher-test";

#[cfg(any(target_os = "linux", target_os = "windows"))]
const TEST_TIMEOUT_SECS: u64 = 30;

#[cfg(any(target_os = "macos", target_os = "linux", test))]
fn not_in_range(ssid: &str) -> String {
    format!("{ssid} is not in range of this computer.")
}

#[cfg(any(target_os = "macos", target_os = "linux", target_os = "windows", test))]
fn refused(ssid: &str, detail: &str) -> String {
    if detail.is_empty() {
        format!("{ssid} refused this computer — check the password.")
    } else {
        format!("{ssid} refused this computer — check the password. ({detail})")
    }
}

#[cfg(target_os = "macos")]
async fn test_wifi_macos(ssid: &str, password: &str) -> Result<String, String> {
    let dev = wifi_device_macos().await;
    let out = tokio::process::Command::new("networksetup")
        .arg("-setairportnetwork")
        .arg(&dev)
        .arg(ssid)
        .args((!password.is_empty()).then_some(password))
        .output()
        .await
        .map_err(|e| format!("Failed to run networksetup: {e}"))?;
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    macos_join_result(ssid, said.trim())
}

#[cfg(any(target_os = "macos", test))]
fn macos_join_result(ssid: &str, said: &str) -> Result<String, String> {
    if said.is_empty() {
        return Ok(format!(
            "Joined {ssid}. This Mac stays on it — switch back in WiFi settings if you need another network."
        ));
    }
    if said.starts_with("Could not find network") {
        return Err(not_in_range(ssid));
    }
    let detail = said
        .lines()
        .find_map(|l| l.trim().strip_prefix("Error:"))
        .map(str::trim)
        .unwrap_or("");
    Err(refused(ssid, detail))
}

#[cfg(target_os = "linux")]
async fn test_wifi_linux(ssid: &str, password: &str) -> Result<String, String> {
    let active = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "NAME,TYPE", "connection", "show", "--active"])
        .output()
        .await
        .map_err(|e| format!("Failed to run nmcli: {e}"))?;
    let previous = active_wifi_connection(&String::from_utf8_lossy(&active.stdout));

    let wait = TEST_TIMEOUT_SECS.to_string();
    let secret = (!password.is_empty())
        .then_some(["password", password])
        .into_iter()
        .flatten();
    let out = tokio::process::Command::new("nmcli")
        .args(["--wait", wait.as_str(), "device", "wifi", "connect", ssid])
        .args(secret)
        .args(["name", TEST_PROFILE])
        .output()
        .await
        .map_err(|e| format!("Failed to run nmcli: {e}"))?;

    let _ = tokio::process::Command::new("nmcli")
        .args(["connection", "delete", TEST_PROFILE])
        .output()
        .await;
    if let Some(prev) = &previous {
        let _ = tokio::process::Command::new("nmcli")
            .args(["connection", "up", prev.as_str()])
            .output()
            .await;
    }

    if out.status.success() {
        return Ok(match previous {
            Some(prev) => format!("Joined {ssid}, then switched back to {prev}."),
            None => format!("Joined {ssid}."),
        });
    }
    Err(nmcli_failure(
        ssid,
        String::from_utf8_lossy(&out.stderr).trim(),
    ))
}

#[cfg(any(target_os = "linux", test))]
fn active_wifi_connection(terse: &str) -> Option<String> {
    terse
        .lines()
        .filter_map(|l| l.rsplit_once(':'))
        .find(|(_, kind)| *kind == "802-11-wireless")
        .map(|(name, _)| name.replace("\\:", ":"))
}

#[cfg(any(target_os = "linux", test))]
fn nmcli_failure(ssid: &str, stderr: &str) -> String {
    if stderr.contains("No network with SSID") {
        return not_in_range(ssid);
    }
    refused(ssid, stderr.trim_start_matches("Error:").trim())
}

#[cfg(target_os = "windows")]
async fn test_wifi_windows(ssid: &str, password: &str) -> Result<String, String> {
    let previous = interface_field(&netsh_interfaces().await, "Profile");

    let path = std::env::temp_dir().join(format!("{TEST_PROFILE}.xml"));
    std::fs::write(&path, windows_profile_xml(ssid, password))
        .map_err(|e| format!("Failed to write a WiFi profile: {e}"))?;
    let added = hidden_command("netsh")
        .args(["wlan", "add", "profile", "user=current"])
        .raw_arg(format!("filename=\"{}\"", path.display()))
        .output()
        .await;
    let _ = std::fs::remove_file(&path);
    let added = added.map_err(|e| format!("Failed to run netsh: {e}"))?;
    if !added.status.success() {
        return Err(refused(ssid, String::from_utf8_lossy(&added.stdout).trim()));
    }

    let _ = hidden_command("netsh")
        .args(["wlan", "connect"])
        .raw_arg(format!("name=\"{TEST_PROFILE}\""))
        .output()
        .await;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(TEST_TIMEOUT_SECS);
    let joined = loop {
        if interface_field(&netsh_interfaces().await, "SSID").as_deref() == Some(ssid) {
            break true;
        }
        if std::time::Instant::now() >= deadline {
            break false;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    };

    let _ = hidden_command("netsh")
        .args(["wlan", "delete", "profile"])
        .raw_arg(format!("name=\"{TEST_PROFILE}\""))
        .output()
        .await;
    if let Some(prev) = &previous {
        let _ = hidden_command("netsh")
            .args(["wlan", "connect"])
            .raw_arg(format!("name=\"{prev}\""))
            .output()
            .await;
    }

    match (joined, previous) {
        (true, Some(prev)) => Ok(format!("Joined {ssid}, then switched back to {prev}.")),
        (true, None) => Ok(format!("Joined {ssid}.")),
        (false, _) => Err(format!(
            "{ssid} did not accept this computer — check the password, and that the network is in range."
        )),
    }
}

#[cfg(target_os = "windows")]
async fn netsh_interfaces() -> String {
    hidden_command("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
        .await
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default()
}

#[cfg(any(target_os = "windows", test))]
fn interface_field(text: &str, key: &str) -> Option<String> {
    text.lines()
        .filter_map(|l| l.split_once(':'))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

#[cfg(any(target_os = "windows", test))]
fn windows_profile_xml(ssid: &str, password: &str) -> String {
    let hex: String = ssid.bytes().map(|b| format!("{b:02X}")).collect();
    let raw_psk = password.len() == 64 && password.chars().all(|c| c.is_ascii_hexdigit());
    let security = if password.is_empty() {
        "<authEncryption><authentication>open</authentication><encryption>none</encryption><useOneX>false</useOneX></authEncryption>".to_string()
    } else {
        format!(
            "<authEncryption><authentication>WPA2PSK</authentication><encryption>AES</encryption><useOneX>false</useOneX></authEncryption><sharedKey><keyType>{}</keyType><protected>false</protected><keyMaterial>{}</keyMaterial></sharedKey>",
            if raw_psk { "networkKey" } else { "passPhrase" },
            xml_escape(password)
        )
    };
    format!(
        "<?xml version=\"1.0\"?><WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\"><name>{TEST_PROFILE}</name><SSIDConfig><SSID><hex>{hex}</hex></SSID></SSIDConfig><connectionType>ESS</connectionType><connectionMode>manual</connectionMode><MSM><security>{security}</security></MSM></WLANProfile>"
    )
}

#[cfg(any(target_os = "windows", test))]
fn xml_escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&apos;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(target_os = "windows")]
async fn list_wifi_windows() -> Result<WifiNetworks, String> {
    let known = hidden_command("netsh")
        .args(["wlan", "show", "profiles"])
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| {
                    l.split_once(':')
                        .filter(|(k, _)| k.contains("All User Profile"))
                })
                .map(|(_, v)| v.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let current = hidden_command("netsh")
        .args(["wlan", "show", "interfaces"])
        .output()
        .await
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .filter_map(|l| l.split_once(':'))
                .find(|(k, _)| {
                    let k = k.trim();
                    k == "SSID"
                })
                .map(|(_, v)| v.trim().to_string())
        })
        .filter(|s| !s.is_empty());

    let country = user_locale_windows()
        .await
        .and_then(|l| region_from_locale(&l));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

#[cfg(target_os = "windows")]
async fn user_locale_windows() -> Option<String> {
    if let Ok(o) = hidden_command("powershell")
        .args(["-NoProfile", "-Command", "(Get-Culture).Name"])
        .output()
        .await
    {
        let name = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .ok()
        .map(|l| l.split('.').next().unwrap_or(&l).to_string())
}

#[cfg(target_os = "macos")]
pub(crate) async fn wifi_device_macos() -> String {
    if let Ok(out) = tokio::process::Command::new("networksetup")
        .arg("-listallhardwareports")
        .output()
        .await
    {
        let text = String::from_utf8_lossy(&out.stdout);
        let mut wifi = false;
        for line in text.lines() {
            if line.contains("Wi-Fi") || line.contains("AirPort") {
                wifi = true;
            } else if wifi {
                if let Some(dev) = line.strip_prefix("Device: ") {
                    return dev.trim().to_string();
                }
            }
        }
    }
    "en0".to_string()
}

#[cfg(target_os = "macos")]
async fn list_wifi_macos() -> Result<WifiNetworks, String> {
    let dev = wifi_device_macos().await;

    let current = tokio::process::Command::new("networksetup")
        .args(["-getairportnetwork", &dev])
        .output()
        .await
        .ok()
        .and_then(|o| {
            String::from_utf8_lossy(&o.stdout)
                .split_once("Current Wi-Fi Network: ")
                .map(|(_, name)| name.trim().to_string())
        })
        .filter(|s| !s.is_empty());

    let known = tokio::process::Command::new("networksetup")
        .args(["-listpreferredwirelessnetworks", &dev])
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1)
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let country = tokio::process::Command::new("defaults")
        .args(["read", "-g", "AppleLocale"])
        .output()
        .await
        .ok()
        .and_then(|o| region_from_locale(String::from_utf8_lossy(&o.stdout).trim()));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

#[cfg(target_os = "linux")]
async fn list_wifi_linux() -> Result<WifiNetworks, String> {
    let out = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "ACTIVE,SSID", "dev", "wifi"])
        .output()
        .await
        .map_err(|e| format!("Failed to run nmcli: {e}"))?;

    let mut current = None;
    let mut known = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let (active, ssid) = line.split_once(':').unwrap_or(("no", line));
        let ssid = ssid.trim();
        if ssid.is_empty() {
            continue;
        }
        if active == "yes" {
            current = Some(ssid.to_string());
        }
        if !known.iter().any(|s| s == ssid) {
            known.push(ssid.to_string());
        }
    }

    let country = std::env::var("LANG")
        .or_else(|_| std::env::var("LC_ALL"))
        .ok()
        .and_then(|l| region_from_locale(l.split('.').next().unwrap_or(&l)));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_from_locale_parses() {
        assert_eq!(region_from_locale("en_US"), Some("US".into()));
        assert_eq!(region_from_locale("ka_GE"), Some("GE".into()));
        assert_eq!(region_from_locale("en_US@rg=uszzzz"), Some("US".into()));
        assert_eq!(region_from_locale("zh-Hans-CN"), Some("CN".into()));
        assert_eq!(region_from_locale("en"), None);
        assert_eq!(region_from_locale("C"), None);
    }

    #[test]
    fn macos_join_result_reads_networksetup() {
        assert!(macos_join_result("home", "").is_ok());
        assert_eq!(
            macos_join_result("home", "Could not find network home."),
            Err("home is not in range of this computer.".into())
        );
        assert_eq!(
            macos_join_result(
                "home",
                "Failed to join network home.\nError: -3905  The operation couldn’t be completed."
            ),
            Err("home refused this computer — check the password. (-3905  The operation couldn’t be completed.)".into())
        );
    }

    #[test]
    fn active_wifi_connection_picks_the_wireless_one() {
        let terse = "Wired connection 1:802-3-ethernet\nCafe\\: Guest:802-11-wireless\nlo:loopback";
        assert_eq!(active_wifi_connection(terse), Some("Cafe: Guest".into()));
        assert_eq!(active_wifi_connection("lo:loopback"), None);
    }

    #[test]
    fn nmcli_failure_tells_range_from_refusal() {
        assert_eq!(
            nmcli_failure("home", "Error: No network with SSID 'home' found."),
            "home is not in range of this computer."
        );
        assert_eq!(
            nmcli_failure(
                "home",
                "Error: Connection activation failed: Secrets were required, but not provided."
            ),
            "home refused this computer — check the password. (Connection activation failed: Secrets were required, but not provided.)"
        );
    }

    #[test]
    fn interface_field_reads_netsh() {
        let text = "    Name                   : Wi-Fi\n    State                  : connected\n    SSID                   : Tenda_DBDAA0\n    BSSID                  : cc:2d:21:db:da:a4\n    Profile                : Tenda_DBDAA0 2\n";
        assert_eq!(interface_field(text, "SSID"), Some("Tenda_DBDAA0".into()));
        assert_eq!(
            interface_field(text, "Profile"),
            Some("Tenda_DBDAA0 2".into())
        );
        assert_eq!(interface_field("    State : disconnected\n", "SSID"), None);
    }

    #[test]
    fn windows_profile_escapes_the_password_and_encodes_the_ssid() {
        let xml = windows_profile_xml("A&B", "p<ss>\"w'd&");
        assert!(xml.contains("<hex>412642</hex>"));
        assert!(xml.contains("<keyType>passPhrase</keyType>"));
        assert!(xml.contains("<keyMaterial>p&lt;ss&gt;&quot;w&apos;d&amp;</keyMaterial>"));
        assert!(xml.contains("<name>aircast-flasher-test</name>"));

        let raw = "a".repeat(64);
        assert!(windows_profile_xml("x", &raw).contains("<keyType>networkKey</keyType>"));

        let open = windows_profile_xml("x", "");
        assert!(open.contains("<authentication>open</authentication>"));
        assert!(!open.contains("sharedKey"));
    }
}
