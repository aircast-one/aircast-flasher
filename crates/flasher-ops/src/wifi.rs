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
}
