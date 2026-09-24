use super::format_bytes;
use super::BlockDevice;

// ---------------------------------------------------------------------------
// Command: list_block_devices
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_block_devices() -> Result<Vec<BlockDevice>, String> {
    #[cfg(target_os = "macos")]
    {
        list_block_devices_macos().await
    }
    #[cfg(target_os = "linux")]
    {
        list_block_devices_linux().await
    }
    #[cfg(target_os = "windows")]
    {
        list_block_devices_windows().await
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        Err("Disk detection not supported on this platform".to_string())
    }
}

#[cfg(target_os = "windows")]
async fn list_block_devices_windows() -> Result<Vec<BlockDevice>, String> {
    // Enumeration touches the kernel; run it off the async runtime.
    let disks = tokio::task::spawn_blocking(flasher_core::win::list_disks)
        .await
        .map_err(|e| format!("Disk enumeration task failed: {e}"))??;

    let mut devices = Vec::new();
    for d in disks {
        // Same guard as macOS/Linux: only removable, non-system, <=2TB targets.
        if !d.removable || d.is_system || d.size == 0 || d.size > 2_000_000_000_000 {
            continue;
        }
        let name = if d.bus.is_empty() || d.bus == "Other" {
            d.model.clone()
        } else {
            format!("{} ({})", d.model, d.bus)
        };
        devices.push(BlockDevice {
            path: d.id,
            name,
            size: d.size,
            size_human: format_bytes(d.size),
            removable: true,
            // Windows auto-mounts volumes; we don't surface mount points here.
            mounted: false,
            mount_points: vec![],
        });
    }
    Ok(devices)
}

#[cfg(target_os = "macos")]
async fn list_block_devices_macos() -> Result<Vec<BlockDevice>, String> {
    let output = tokio::process::Command::new("diskutil")
        .args(["list", "-plist", "physical"])
        .output()
        .await
        .map_err(|e| format!("Failed to run diskutil list: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "diskutil list failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let plist_value: plist::Value = plist::from_bytes(&output.stdout)
        .map_err(|e| format!("Failed to parse diskutil plist: {e}"))?;
    let disk_ids = plist_value
        .as_dictionary()
        .and_then(|d| d.get("AllDisksAndPartitions"))
        .and_then(|v| v.as_array())
        .ok_or("Unexpected diskutil output format")?;

    let mut devices = Vec::new();
    for disk_entry in disk_ids {
        let dict = match disk_entry.as_dictionary() {
            Some(d) => d,
            None => continue,
        };
        let device_id = match dict.get("DeviceIdentifier").and_then(|v| v.as_string()) {
            Some(id) => id.to_string(),
            None => continue,
        };
        let path = format!("/dev/{device_id}");

        let info_output = tokio::process::Command::new("diskutil")
            .args(["info", "-plist", &path])
            .output()
            .await
            .map_err(|e| format!("Failed to run diskutil info: {e}"))?;
        if !info_output.status.success() {
            continue;
        }
        let info_plist: plist::Value = match plist::from_bytes(&info_output.stdout) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let info = match info_plist.as_dictionary() {
            Some(d) => d,
            None => continue,
        };

        let size = match info.get("TotalSize").and_then(|v| v.as_unsigned_integer()) {
            Some(s) if s > 0 && s <= 2_000_000_000_000 => s,
            _ => continue, // unknown, zero, or >2TB (likely an external HDD/SSD)
        };
        let name = info
            .get("MediaName")
            .and_then(|v| v.as_string())
            .unwrap_or("Unknown Device")
            .to_string();
        // The removable flag — NOT `Internal` — is the system-disk guard: the
        // built-in SD reader reports `Internal = true` but `Ejectable = true`,
        // while the boot disk is `Ejectable = false`. Filtering on `Internal`
        // would wrongly hide built-in card readers.
        let removable = info
            .get("Ejectable")
            .and_then(|v| v.as_boolean())
            .or_else(|| {
                info.get("RemovableMediaOrExternalDevice")
                    .and_then(|v| v.as_boolean())
            })
            .unwrap_or(false);
        if !removable {
            continue;
        }

        let mut mount_points: Vec<String> = Vec::new();
        if let Some(mp) = info.get("MountPoint").and_then(|v| v.as_string()) {
            if !mp.is_empty() {
                mount_points.push(mp.to_string());
            }
        }
        if let Some(parts) = dict.get("Partitions").and_then(|v| v.as_array()) {
            for part in parts {
                if let Some(mp) = part
                    .as_dictionary()
                    .and_then(|p| p.get("MountPoint"))
                    .and_then(|v| v.as_string())
                {
                    if !mp.is_empty() {
                        mount_points.push(mp.to_string());
                    }
                }
            }
        }
        let mounted = !mount_points.is_empty();

        devices.push(BlockDevice {
            path,
            name,
            size,
            size_human: format_bytes(size),
            removable,
            mounted,
            mount_points,
        });
    }
    Ok(devices)
}

#[cfg(target_os = "linux")]
async fn list_block_devices_linux() -> Result<Vec<BlockDevice>, String> {
    let output = tokio::process::Command::new("lsblk")
        .args([
            "--json",
            "--output",
            "NAME,SIZE,RM,TYPE,MOUNTPOINT,MODEL",
            "--bytes",
        ])
        .output()
        .await
        .map_err(|e| format!("Failed to run lsblk: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "lsblk failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let json: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| format!("Failed to parse lsblk output: {e}"))?;
    let blockdevices = json
        .get("blockdevices")
        .and_then(|v| v.as_array())
        .ok_or("Unexpected lsblk output format")?;

    let mut devices = Vec::new();
    for dev in blockdevices {
        let dev_type = dev.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let removable = dev.get("rm").and_then(|v| v.as_bool()).unwrap_or(false);
        if dev_type != "disk" || !removable {
            continue;
        }
        let name_str = dev.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let path = format!("/dev/{name_str}");
        let size = dev
            .get("size")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
            })
            .unwrap_or(0);
        if size == 0 || size > 2_000_000_000_000 {
            continue;
        }
        let model = dev
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown Device")
            .trim()
            .to_string();
        let mountpoint = dev.get("mountpoint").and_then(|v| v.as_str()).unwrap_or("");
        let mounted = !mountpoint.is_empty();
        let mount_points = if mounted {
            vec![mountpoint.to_string()]
        } else {
            vec![]
        };

        devices.push(BlockDevice {
            path,
            name: model,
            size,
            size_human: format_bytes(size),
            removable: true,
            mounted,
            mount_points,
        });
    }
    Ok(devices)
}
