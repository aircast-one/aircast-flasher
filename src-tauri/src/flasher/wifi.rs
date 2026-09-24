#[cfg(target_os = "windows")]
use super::hidden_command;
use super::Serialize;

// ---------------------------------------------------------------------------
// Command: list_wifi_networks (so the user can pick instead of typing)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
pub struct WifiNetworks {
    /// The currently-joined SSID, if any (used to prefill the form).
    pub current: Option<String>,
    /// Selectable SSIDs: known/preferred networks on macOS (no Location
    /// permission needed); a live scan on Linux.
    pub known: Vec<String>,
    /// Detected 2-letter WiFi regulatory domain from the system locale, so the
    /// user doesn't have to type it. `None` if it can't be determined.
    pub country: Option<String>,
}

/// Extract the 2-letter region from a locale like `en_US`, `ka_GE`,
/// `en_US@rg=...`, or `zh-Hans-CN`. Returns uppercase, or None.
fn region_from_locale(locale: &str) -> Option<String> {
    let base = locale.split('@').next().unwrap_or(locale);
    let parts: Vec<&str> = base.split(['_', '-']).filter(|s| !s.is_empty()).collect();
    if parts.len() < 2 {
        return None; // language only, no region
    }
    let region = parts.last()?;
    if region.len() == 2 && region.chars().all(|c| c.is_ascii_alphabetic()) {
        Some(region.to_uppercase())
    } else {
        None
    }
}

#[tauri::command]
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

#[cfg(target_os = "windows")]
async fn list_wifi_windows() -> Result<WifiNetworks, String> {
    // Known/saved networks: `netsh wlan show profiles` lists
    //   "    All User Profile     : <NAME>"
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

    // Current SSID: `netsh wlan show interfaces` has a line
    //   "    SSID                   : <NAME>"
    // Careful: there is also a "BSSID" line — match the SSID line specifically.
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

    // Region from the user locale, e.g. "en-US" → "US".
    let country = user_locale_windows()
        .await
        .and_then(|l| region_from_locale(&l));

    Ok(WifiNetworks {
        current,
        known,
        country,
    })
}

/// Best-effort user locale name on Windows (e.g. "en-US"), via PowerShell's
/// Get-Culture, falling back to the LANG/locale env if PowerShell is missing.
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
    // Find the Wi-Fi hardware port's device (usually en0), fall back to en0.
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

    // Currently-joined SSID (may be "You are not associated…").
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

    // Preferred/known networks — no Location permission required.
    let known = tokio::process::Command::new("networksetup")
        .args(["-listpreferredwirelessnetworks", &dev])
        .output()
        .await
        .map(|o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .skip(1) // "Preferred networks on enN:"
                .map(|l| l.trim().to_string())
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    // Region from the system locale (e.g. AppleLocale "ka_GE" → "GE").
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
    // Live scan via NetworkManager. `-t` terse, `-f` fields.
    let out = tokio::process::Command::new("nmcli")
        .args(["-t", "-f", "ACTIVE,SSID", "dev", "wifi"])
        .output()
        .await
        .map_err(|e| format!("Failed to run nmcli: {e}"))?;

    let mut current = None;
    let mut known = Vec::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        // Format: "yes:MySSID" / "no:OtherSSID"
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

    // Region from the locale env (e.g. LANG "en_US.UTF-8" → "US").
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

// ---------------------------------------------------------------------------

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
}
