//! Windows raw-disk support for the Aircast Flasher.
//!
//! Pure Rust over the `windows` crate Win32 bindings — no Tauri, no app deps —
//! so it cross-checks for `x86_64-pc-windows-msvc` from any host. Enumerates
//! physical drives, identifies the removable USB/SD targets, marks the system
//! disk off-limits, and writes a raw image straight to `\\.\PhysicalDriveN`.

#![cfg(target_os = "windows")]

use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{
    BusTypeMmc, BusTypeSata, BusTypeSd, BusTypeUsb, CreateFileW, FlushFileBuffers, ReadFile,
    SetFilePointerEx, WriteFile, FILE_BEGIN, FILE_CURRENT, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_FLAG_NO_BUFFERING, FILE_FLAG_WRITE_THROUGH, FILE_GENERIC_READ, FILE_SHARE_MODE,
    FILE_SHARE_READ, FILE_SHARE_WRITE, IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS, OPEN_EXISTING,
};
use windows::Win32::System::Ioctl::{
    PropertyStandardQuery, StorageDeviceProperty, DISK_EXTENT, DISK_GEOMETRY_EX,
    FSCTL_DISMOUNT_VOLUME, FSCTL_LOCK_VOLUME, FSCTL_UNLOCK_VOLUME, IOCTL_DISK_DELETE_DRIVE_LAYOUT,
    IOCTL_DISK_GET_DRIVE_GEOMETRY_EX, IOCTL_DISK_UPDATE_PROPERTIES, IOCTL_STORAGE_QUERY_PROPERTY,
    STORAGE_DEVICE_DESCRIPTOR, STORAGE_PROPERTY_QUERY, VOLUME_DISK_EXTENTS,
};
use windows::Win32::System::IO::DeviceIoControl;

/// A physical drive on the system.
pub struct WinDisk {
    /// Device id, e.g. `\\.\PhysicalDrive2`.
    pub id: String,
    /// Human-readable model (vendor + product), e.g. "SanDisk Ultra".
    pub model: String,
    /// Total size in bytes.
    pub size: u64,
    /// Whether the medium reports as removable.
    pub removable: bool,
    /// Bus type bucket: "USB", "SD", or "Other".
    pub bus: String,
    /// True if this is the disk the OS booted from — never a flash target.
    pub is_system: bool,
}

/// RAII wrapper that closes a Win32 handle on drop.
struct Handle(HANDLE);

impl Handle {
    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// Encode a `&str` as a NUL-terminated UTF-16 buffer for the wide Win32 APIs.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Open a device/volume path with explicit access/share/flags; `None` on failure
/// (e.g. the drive doesn't exist or we lack rights).
fn open_handle(path: &str, access: u32, share: u32, flags: u32) -> Option<Handle> {
    let wpath = wide(path);
    unsafe {
        let h = CreateFileW(
            PCWSTR(wpath.as_ptr()),
            access,
            FILE_SHARE_MODE(share),
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(flags),
            None,
        );
        match h {
            Ok(handle) if handle != INVALID_HANDLE_VALUE => Some(Handle(handle)),
            _ => None,
        }
    }
}

/// Combined FILE_SHARE_READ | FILE_SHARE_WRITE as a raw u32.
fn share_rw() -> u32 {
    (FILE_SHARE_READ | FILE_SHARE_WRITE).0
}

/// Query the disk number backing a given volume handle, if any.
fn volume_disk_number(handle: HANDLE) -> Option<u32> {
    // A volume can span multiple extents; size the buffer for the header plus a
    // generous number of inline DISK_EXTENTs (single-disk volumes need just one).
    let mut buf = vec![
        0u8;
        std::mem::size_of::<VOLUME_DISK_EXTENTS>()
            + 16 * std::mem::size_of::<DISK_EXTENT>()
    ];
    let mut returned: u32 = 0;
    unsafe {
        let ok = DeviceIoControl(
            handle,
            IOCTL_VOLUME_GET_VOLUME_DISK_EXTENTS,
            None,
            0,
            Some(buf.as_mut_ptr() as *mut _),
            buf.len() as u32,
            Some(&mut returned),
            None,
        );
        if ok.is_err() {
            return None;
        }
        let ext = &*(buf.as_ptr() as *const VOLUME_DISK_EXTENTS);
        if ext.NumberOfDiskExtents == 0 {
            return None;
        }
        // Extents array is inline immediately after the header.
        let first = &*(ext.Extents.as_ptr());
        Some(first.DiskNumber)
    }
}

/// Determine which PhysicalDriveN the OS booted from, via the system drive volume.
fn system_disk_number() -> Option<u32> {
    let sys = std::env::var("SystemDrive").unwrap_or_else(|_| "C:".to_string());
    let letter = sys.trim_end_matches(['\\', '/']);
    let vol = format!("\\\\.\\{letter}");
    let handle = open_handle(&vol, 0, share_rw(), 0)?;
    volume_disk_number(handle.raw())
}

/// Query a STORAGE_DEVICE_DESCRIPTOR for a physical-drive handle: (bus, removable, model).
fn query_device_descriptor(handle: HANDLE) -> Option<(String, bool, String)> {
    let query = STORAGE_PROPERTY_QUERY {
        PropertyId: StorageDeviceProperty,
        QueryType: PropertyStandardQuery,
        AdditionalParameters: [0],
    };
    // Generous fixed buffer; vendor/product strings live at byte offsets within.
    let mut buf = vec![0u8; 1024];
    let mut returned: u32 = 0;
    unsafe {
        let ok = DeviceIoControl(
            handle,
            IOCTL_STORAGE_QUERY_PROPERTY,
            Some(&query as *const _ as *const _),
            std::mem::size_of::<STORAGE_PROPERTY_QUERY>() as u32,
            Some(buf.as_mut_ptr() as *mut _),
            buf.len() as u32,
            Some(&mut returned),
            None,
        );
        if ok.is_err() || (returned as usize) < std::mem::size_of::<STORAGE_DEVICE_DESCRIPTOR>() {
            return None;
        }
        let desc = &*(buf.as_ptr() as *const STORAGE_DEVICE_DESCRIPTOR);

        let bus = match desc.BusType {
            t if t == BusTypeUsb => "USB",
            t if t == BusTypeSd => "SD",
            t if t == BusTypeMmc => "SD",
            t if t == BusTypeSata => "Other",
            _ => "Other",
        }
        .to_string();

        // BOOLEAN is a u8 wrapper.
        let removable = desc.RemovableMedia.0 != 0;

        let vendor = read_offset_string(&buf, desc.VendorIdOffset as usize);
        let product = read_offset_string(&buf, desc.ProductIdOffset as usize);
        let model = match (vendor.is_empty(), product.is_empty()) {
            (false, false) => format!("{vendor} {product}"),
            (true, false) => product,
            (false, true) => vendor,
            (true, true) => "Unknown Device".to_string(),
        };
        Some((bus, removable, model.trim().to_string()))
    }
}

/// Read a NUL-terminated ASCII string at `offset` within the descriptor buffer.
fn read_offset_string(buf: &[u8], offset: usize) -> String {
    if offset == 0 || offset >= buf.len() {
        return String::new();
    }
    let bytes = &buf[offset..];
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

/// Query the size in bytes of a physical-drive handle. Uses
/// IOCTL_DISK_GET_DRIVE_GEOMETRY_EX (`FILE_ANY_ACCESS`) rather than
/// IOCTL_DISK_GET_LENGTH_INFO (`FILE_READ_ACCESS`) so it succeeds on the
/// zero-access handle [`list_disks`] opens; the latter returns ACCESS_DENIED
/// there, leaving every disk at size 0 and filtered out as a target.
fn query_disk_length(handle: HANDLE) -> Option<u64> {
    let mut geometry = DISK_GEOMETRY_EX::default();
    let mut returned: u32 = 0;
    unsafe {
        let ok = DeviceIoControl(
            handle,
            IOCTL_DISK_GET_DRIVE_GEOMETRY_EX,
            None,
            0,
            Some(&mut geometry as *mut _ as *mut _),
            std::mem::size_of::<DISK_GEOMETRY_EX>() as u32,
            Some(&mut returned),
            None,
        );
        if ok.is_err() {
            return None;
        }
    }
    Some(geometry.DiskSize as u64)
}

/// Enumerate `\\.\PhysicalDrive0..=31`, skipping ones that can't be opened.
pub fn list_disks() -> Result<Vec<WinDisk>, String> {
    let system = system_disk_number();
    let mut disks = Vec::new();

    for n in 0u32..=31 {
        let id = format!("\\\\.\\PhysicalDrive{n}");
        // Desired access 0: query metadata without needing write/admin rights.
        let handle = match open_handle(&id, 0, share_rw(), 0) {
            Some(h) => h,
            None => continue,
        };

        let (bus, mut removable, model) = query_device_descriptor(handle.raw())
            .unwrap_or_else(|| ("Other".to_string(), false, "Unknown Device".to_string()));
        let size = query_disk_length(handle.raw()).unwrap_or(0);
        let is_system = system == Some(n);
        if is_system {
            // Never let the boot disk look like a removable flash target.
            removable = false;
        }

        disks.push(WinDisk {
            id,
            model,
            size,
            removable,
            bus,
            is_system,
        });
    }

    Ok(disks)
}

/// Parse the trailing drive number from a `\\.\PhysicalDriveN` id.
fn drive_number(device: &str) -> Result<u32, String> {
    let digits: String = device
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if digits.is_empty() {
        return Err(format!("Invalid physical drive id: {device}"));
    }
    digits
        .parse::<u32>()
        .map_err(|e| format!("Invalid drive number in {device}: {e}"))
}

/// Lock + dismount every mounted volume that lives on `disk_no`, keeping the
/// handles open (returned) so the locks persist for the duration of the write.
fn lock_volumes_on_disk(disk_no: u32) -> Vec<Handle> {
    let read_write = (FILE_GENERIC_READ.0) | (GENERIC_WRITE.0);
    let mut locked = Vec::new();
    for letter in b'A'..=b'Z' {
        let vol = format!("\\\\.\\{}:", letter as char);
        let handle = match open_handle(&vol, read_write, share_rw(), 0) {
            Some(h) => h,
            None => continue,
        };
        if volume_disk_number(handle.raw()) != Some(disk_no) {
            continue;
        }
        let mut returned: u32 = 0;
        unsafe {
            // Best-effort: a failed lock just means we proceed; the physical
            // write still goes through once the volume is dismounted.
            let _ = DeviceIoControl(
                handle.raw(),
                FSCTL_LOCK_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut returned),
                None,
            );
            let _ = DeviceIoControl(
                handle.raw(),
                FSCTL_DISMOUNT_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut returned),
                None,
            );
        }
        locked.push(handle);
    }
    locked
}

/// Explicitly unlock the held volume handles (Drop would close them anyway, but
/// unlocking first lets Windows remount cleanly).
fn unlock_volumes(handles: &[Handle]) {
    for h in handles {
        let mut returned: u32 = 0;
        unsafe {
            let _ = DeviceIoControl(
                h.raw(),
                FSCTL_UNLOCK_VOLUME,
                None,
                0,
                None,
                0,
                Some(&mut returned),
                None,
            );
        }
    }
}

/// Drop the disk's partition table.
///
/// Windows refuses raw writes to any sector claimed by a volume, and it only
/// gives up that claim when the partition disappears — which is why writing over
/// a card that still carries an old layout fails with ACCESS_DENIED until the
/// user deletes the partitions by hand in Disk Management. Deleting the layout
/// ourselves (what Raspberry Pi Imager does) releases every volume on the disk,
/// lettered or not, so the whole medium is writable. Best-effort: a blank card
/// has no layout to delete, and the write is what actually reports failure.
fn delete_drive_layout(handle: HANDLE) {
    let mut returned: u32 = 0;
    unsafe {
        let _ = DeviceIoControl(
            handle,
            IOCTL_DISK_DELETE_DRIVE_LAYOUT,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        );
        // Make the kernel re-read the (now empty) layout and tear the volumes down.
        let _ = DeviceIoControl(
            handle,
            IOCTL_DISK_UPDATE_PROPERTIES,
            None,
            0,
            None,
            0,
            Some(&mut returned),
            None,
        );
    }
}

/// An opened `\\.\PhysicalDriveN` handle the engine drives directly.
///
/// Holds the physical-drive handle plus the locked/dismounted child-volume
/// handles for the lifetime of the flash, so Windows can't remount and dirty the
/// boot partition mid-flight. Implements `Read + Write + Seek` over the raw
/// handle; the caller wraps it in [`crate::AlignedDevice`] to satisfy the
/// `FILE_FLAG_NO_BUFFERING` sector-alignment requirement.
pub struct WinDevice {
    disk: Handle,
    /// Disk size in bytes, from IOCTL_DISK_GET_DRIVE_GEOMETRY_EX. A raw
    /// PhysicalDrive handle rejects `SetFilePointerEx(FILE_END)` with
    /// ERROR_INVALID_FUNCTION, so end-relative seeks are resolved against this.
    size: u64,
    /// Locked volume handles, kept open (and unlocked on drop) for the flash.
    locked: Vec<Handle>,
}

// SAFETY: the Win32 HANDLEs are owned exclusively by this struct and only
// touched through its own &mut methods; ownership moves to the helper thread
// exactly once.
unsafe impl Send for WinDevice {}

impl Drop for WinDevice {
    fn drop(&mut self) {
        // Unlock before the handles close so Windows can remount cleanly.
        unlock_volumes(&self.locked);
    }
}

/// Open `\\.\PhysicalDriveN` for the engine: lock+dismount child volumes, open
/// the drive with no buffering + write-through, then drop its partition table so
/// no leftover volume can veto a write.
///
/// Requires the calling process to be elevated. The returned [`WinDevice`] is
/// `Read + Write + Seek` and must be wrapped in [`crate::AlignedDevice`].
pub fn open_device(device: &str) -> Result<WinDevice, String> {
    let disk_no = drive_number(device)?;
    let id = format!("\\\\.\\PhysicalDrive{disk_no}");

    // Hold these handles open for the whole flash to keep the volumes locked.
    let locked = lock_volumes_on_disk(disk_no);

    let disk = open_handle(
        &id,
        (GENERIC_READ | GENERIC_WRITE).0,
        share_rw(),
        (FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH).0,
    )
    .ok_or_else(|| format!("Failed to open {id} for read/write (is the app elevated?)"))?;

    delete_drive_layout(disk.raw());

    let size =
        query_disk_length(disk.raw()).ok_or_else(|| format!("Failed to query the size of {id}"))?;

    Ok(WinDevice { disk, size, locked })
}

impl std::io::Read for WinDevice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut bytes_read: u32 = 0;
        unsafe {
            ReadFile(self.disk.raw(), Some(buf), Some(&mut bytes_read), None)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(bytes_read as usize)
    }
}

impl std::io::Write for WinDevice {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let mut bytes_written: u32 = 0;
        unsafe {
            WriteFile(self.disk.raw(), Some(buf), Some(&mut bytes_written), None)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(bytes_written as usize)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        unsafe {
            FlushFileBuffers(self.disk.raw()).map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(())
    }
}

impl std::io::Seek for WinDevice {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        use std::io::SeekFrom;
        let (method, distance) = match pos {
            SeekFrom::Start(n) => (FILE_BEGIN, n as i64),
            SeekFrom::Current(d) => (FILE_CURRENT, d),
            SeekFrom::End(d) => (FILE_BEGIN, self.size as i64 + d),
        };
        let mut new_pos: i64 = 0;
        unsafe {
            SetFilePointerEx(self.disk.raw(), distance, Some(&mut new_pos), method)
                .map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(new_pos as u64)
    }
}

impl crate::engine::BlockDevice for WinDevice {
    fn sync(&mut self) -> std::io::Result<()> {
        unsafe {
            FlushFileBuffers(self.disk.raw()).map_err(|e| std::io::Error::other(e.to_string()))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::drive_number;

    #[test]
    fn drive_number_parses() {
        assert_eq!(drive_number("\\\\.\\PhysicalDrive2"), Ok(2));
        assert_eq!(drive_number("\\\\.\\PhysicalDrive0"), Ok(0));
        assert_eq!(drive_number("\\\\.\\PhysicalDrive31"), Ok(31));
        assert!(drive_number("\\\\.\\PhysicalDrive").is_err());
        assert!(drive_number("garbage").is_err());
    }
}
