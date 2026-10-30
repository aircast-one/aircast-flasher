use std::io::{Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use rockfile::boot::{RkBootEntry, RkBootEntryBytes, RkBootHeader, RkBootHeaderBytes};
use rockusb::device::DeviceIOAsync;
use rockusb::nusb::{Device, Transport};
use rockusb::protocol::ResetOpcode;
use sha2::{Digest, Sha256};

use super::{format_bytes, http, BlockDevice};

const ROCKCHIP_VENDOR: u16 = 0x2207;
const LOADER_BASE: &str =
    "https://raw.githubusercontent.com/radxa/rkbin/69de99907a4b51e4012c704502d234ea3c0f0423/bin/rk35";
const LOADER_SWITCH_TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(250);
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const UNRESPONSIVE: &str = "The board stopped responding over USB. Unplug it, hold its BOOT button while plugging it back in, and try again";
const TARGET_PREFIX: &str = "usb:";
const BOARD_MAKERS: &[&str] = &["radxa", "orange", "xunlong", "rockchip"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Family {
    pub id: &'static str,
    pub chips: &'static str,
    product_id: u16,
    loader: &'static str,
    loader_sha256: &'static str,
}

pub const FAMILIES: &[Family] = &[
    Family {
        id: "rk356x",
        chips: "RK3566/RK3568",
        product_id: 0x350a,
        loader: "rk356x_spl_loader_v1.25.114.bin",
        loader_sha256: "d7db6f39d0b0dd2b5d12e3944d670b2e8ad7ba42029b249f03d9c8ff71bf9bba",
    },
    Family {
        id: "rk3588",
        chips: "RK3588/RK3588S/RK3582",
        product_id: 0x350b,
        loader: "rk3588_spl_loader_v1.22.114.bin",
        loader_sha256: "a18c9a2f50df132242e3fb1d770672d829872a6bfffae1997e64cb6cfeb1d932",
    },
];

pub fn is_target(path: &str) -> bool {
    path.starts_with(TARGET_PREFIX)
}

pub fn family_of_target(path: &str) -> Option<&'static Family> {
    let id = path.strip_prefix(TARGET_PREFIX)?;
    FAMILIES.iter().find(|f| f.id == id)
}

fn family_of_product(product_id: u16) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.product_id == product_id)
}

fn as_target(family: &Family) -> BlockDevice {
    BlockDevice {
        path: format!("{TARGET_PREFIX}{}", family.id),
        name: format!("{} board on USB, built-in storage", family.chips),
        size: 0,
        size_human: "Size checked before writing".to_string(),
        removable: true,
        mounted: false,
        mount_points: vec![],
    }
}

async fn rockchip_devices() -> Vec<nusb::DeviceInfo> {
    nusb::list_devices()
        .await
        .map(|devices| {
            devices
                .filter(|d| d.vendor_id() == ROCKCHIP_VENDOR)
                .collect()
        })
        .unwrap_or_default()
}

pub async fn flashing_boards() -> Vec<BlockDevice> {
    let found: Vec<_> = rockchip_devices()
        .await
        .iter()
        .filter_map(|d| family_of_product(d.product_id()))
        .collect();
    FAMILIES
        .iter()
        .filter(|f| found.contains(f))
        .map(as_target)
        .collect()
}

pub async fn running_boards() -> Vec<String> {
    if cfg!(feature = "no-real-devices") {
        return Vec::new();
    }
    nusb::list_devices()
        .await
        .map(|devices| {
            devices
                .filter(|d| d.vendor_id() != ROCKCHIP_VENDOR)
                .filter_map(|d| {
                    let maker = d.manufacturer_string().unwrap_or_default();
                    let lower = maker.to_lowercase();
                    BOARD_MAKERS
                        .iter()
                        .any(|m| lower.contains(m))
                        .then(|| maker.to_string())
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn loader_file(family: &Family) -> Result<PathBuf, String> {
    let dir = super::paths::cache_dir()?.join("loaders");
    let path = dir.join(family.loader);
    let cached = tokio::fs::read(&path).await.ok();
    if cached
        .as_deref()
        .is_some_and(|b| sha256_hex(b) == family.loader_sha256)
    {
        return Ok(path);
    }
    let url = format!("{LOADER_BASE}/{}", family.loader);
    let bytes = http()
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("Failed to download the {} USB loader: {e}", family.chips))?
        .bytes()
        .await
        .map_err(|e| format!("Failed to download the {} USB loader: {e}", family.chips))?;
    if sha256_hex(&bytes) != family.loader_sha256 {
        return Err(format!(
            "The {} USB loader didn't match its checksum; not using it",
            family.chips
        ));
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    tokio::fs::write(&path, &bytes)
        .await
        .map_err(|e| format!("Failed to save the USB loader: {e}"))?;
    Ok(path)
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn loader_entries(file: &[u8]) -> Result<Vec<(u16, Vec<u8>, u32)>, String> {
    let header_bytes: &RkBootHeaderBytes = file
        .get(..102)
        .and_then(|b| b.try_into().ok())
        .ok_or("The USB loader is too short")?;
    let header = RkBootHeader::from_bytes(header_bytes)
        .ok_or("The USB loader is not a Rockchip boot file")?;
    [(0x471u16, header.entry_471), (0x472u16, header.entry_472)]
        .into_iter()
        .flat_map(|(code, list)| {
            (0..list.count)
                .map(move |i| (code, list.offset as usize + list.size as usize * i as usize))
        })
        .map(|(code, at)| {
            let entry_bytes: &RkBootEntryBytes = file
                .get(at..at + 57)
                .and_then(|b| b.try_into().ok())
                .ok_or("The USB loader has a truncated entry")?;
            let entry = RkBootEntry::from_bytes(entry_bytes);
            let start = entry.data_offset as usize;
            let data = file
                .get(start..start + entry.data_size as usize)
                .ok_or("The USB loader has truncated data")?
                .to_vec();
            Ok((code, data, entry.data_delay))
        })
        .collect()
}

async fn find(family: &Family) -> Option<nusb::DeviceInfo> {
    rockchip_devices()
        .await
        .into_iter()
        .find(|d| d.product_id() == family.product_id)
}

enum Mode {
    Ready(Device),
    NoStorage(Device),
    Maskrom(nusb::DeviceInfo),
}

async fn mode(family: &Family) -> Option<Mode> {
    let info = find(family).await?;
    let mut device = Device::from_usb_device_info(info.clone()).await.ok()?;
    Some(
        match tokio::time::timeout(PROBE_TIMEOUT, device.flash_info()).await {
            Ok(Ok(flash)) if flash.size() > 0 => Mode::Ready(device),
            Ok(Ok(_)) => Mode::NoStorage(device),
            _ => Mode::Maskrom(info),
        },
    )
}

async fn open_device(info: nusb::DeviceInfo) -> Result<Device, String> {
    Device::from_usb_device_info(info)
        .await
        .map_err(|e| format!("Could not open the board over USB: {e:?}"))
}

async fn start_loader(family: &Family, info: nusb::DeviceInfo) -> Result<(), String> {
    let loader = tokio::fs::read(loader_file(family).await?)
        .await
        .map_err(|e| format!("Failed to read the USB loader: {e}"))?;
    let mut device = open_device(info).await?;
    for (code, data, delay_ms) in loader_entries(&loader)? {
        device
            .write_maskrom_area(code, &data)
            .await
            .map_err(|e| format!("{UNRESPONSIVE} ({e:?})"))?;
        tokio::time::sleep(Duration::from_millis(u64::from(delay_ms))).await;
    }
    Ok(())
}

async fn ready(
    family: &Family,
    deadline: tokio::time::Instant,
    loader_sent: bool,
    reset_sent: bool,
) -> Result<Device, String> {
    let (loader_sent, reset_sent) = match mode(family).await {
        Some(Mode::Ready(device)) => return Ok(device),
        Some(Mode::NoStorage(mut device)) if !reset_sent => {
            let _ = device.reset_device(ResetOpcode::Maskrom).await;
            (false, true)
        }
        Some(Mode::Maskrom(info)) if !loader_sent => {
            start_loader(family, info).await?;
            (true, reset_sent)
        }
        _ => (loader_sent, reset_sent),
    };
    if tokio::time::Instant::now() >= deadline {
        return Err(UNRESPONSIVE.to_string());
    }
    tokio::time::sleep(POLL).await;
    Box::pin(ready(family, deadline, loader_sent, reset_sent)).await
}

pub struct UsbDisk {
    io: DeviceIOAsync<Device, Transport>,
    cancel: Arc<AtomicBool>,
}

impl UsbDisk {
    pub fn size(&self) -> u64 {
        self.io.size()
    }

    pub fn size_human(&self) -> String {
        format_bytes(self.size())
    }
}

pub async fn open(family: &Family, cancel: Arc<AtomicBool>) -> Result<UsbDisk, String> {
    find(family).await.ok_or_else(|| {
        format!(
            "No {} board in USB flashing mode is connected",
            family.chips
        )
    })?;
    let deadline = tokio::time::Instant::now() + LOADER_SWITCH_TIMEOUT;
    let device = ready(family, deadline, false, false).await?;
    let io = DeviceIOAsync::new(device)
        .await
        .map_err(|e| format!("Could not read the board's storage size: {e:?}"))?;
    Ok(UsbDisk { io, cancel })
}

pub async fn restart(family: &Family) -> Result<(), String> {
    let info = find(family)
        .await
        .ok_or("The board disconnected before it could restart")?;
    open_device(info)
        .await?
        .reset_device(ResetOpcode::Reset)
        .await
        .map_err(|e| format!("Could not restart the board: {e:?}"))
}

fn cancelled() -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Interrupted, "Cancelled")
}

impl Read for UsbDisk {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        futures::executor::block_on(self.io.read(buf))
    }
}

impl Write for UsbDisk {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.cancel.load(Ordering::SeqCst) {
            return Err(cancelled());
        }
        futures::executor::block_on(self.io.write(buf))
    }

    fn flush(&mut self) -> std::io::Result<()> {
        futures::executor::block_on(self.io.flush())
    }
}

impl Seek for UsbDisk {
    fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
        futures::executor::block_on(self.io.seek(pos))
    }
}

impl flasher_core::engine::BlockDevice for UsbDisk {
    fn sync(&mut self) -> std::io::Result<()> {
        self.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_usb_target_names_its_chip_family() {
        let family = family_of_target("usb:rk356x").unwrap();
        assert_eq!(family.chips, "RK3566/RK3568");
        assert!(is_target("usb:rk3588") && !is_target("/dev/disk4"));
        assert_eq!(family_of_target("usb:rk3399"), None);
        assert_eq!(as_target(family).path, "usb:rk356x");
    }

    #[test]
    fn each_family_maps_its_maskrom_product_id() {
        assert_eq!(family_of_product(0x350a).unwrap().id, "rk356x");
        assert_eq!(family_of_product(0x350b).unwrap().id, "rk3588");
        assert_eq!(family_of_product(0x1234), None);
    }

    #[test]
    fn a_short_or_foreign_file_is_refused_as_a_loader() {
        assert!(loader_entries(&[0u8; 10]).is_err());
        assert!(loader_entries(&[0u8; 200]).is_err());
    }
}
