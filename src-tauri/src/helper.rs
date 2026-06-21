//! The elevated flash helper.
//!
//! `aircast-flasher --flash-helper ...` re-executes this same binary inside one
//! elevated process (Touch ID on macOS, UAC on Windows, pkexec on Linux). That
//! process opens the raw device as root/admin, holds it open for the entire
//! write → verify → customize operation, and runs [`flasher_core::engine::flash`]
//! over it. Holding a single handle for the whole op is the crux of the design:
//! the OS never sees the device closed mid-flight, so it can't auto-mount and
//! dirty the freshly written FAT boot partition.
//!
//! Progress is reported as JSON lines to both the `--progress-file` and stdout:
//! `{"stage":"write|verify|customize","done":N,"total":M}`, with a final
//! `{"stage":"done"}` on success or `{"error":"..."}` on failure. Exit 0 = ok.

use std::io::Write;

use base64::Engine as _;
use flasher_core::engine::{self, FlashParams, FlashStage};
use flasher_core::{AlignedDevice, ProvisionConfig};

/// Entry point for the `--flash-helper` re-exec. Parses args, opens the device,
/// runs the engine, reports progress, and exits the process (never returns).
pub fn run_flash_helper() -> ! {
    let args: Vec<String> = std::env::args().collect();
    let get = |flag: &str| -> Option<String> {
        args.iter()
            .position(|a| a == flag)
            .and_then(|i| args.get(i + 1).cloned())
    };

    let device = get("--device");
    let image = get("--image");
    let progress_file = get("--progress-file");
    let provision_b64 = get("--provision");
    let no_verify = args.iter().any(|a| a == "--no-verify");

    let (device, image, progress_file, provision_b64) =
        match (device, image, progress_file, provision_b64) {
            (Some(d), Some(i), Some(p), Some(c)) => (d, i, p, c),
            _ => {
                eprintln!("flash-helper: missing --device/--image/--progress-file/--provision");
                std::process::exit(2);
            }
        };

    // Append a single JSON line to the progress file AND stdout (best-effort).
    let append = move |line: &str| {
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&progress_file)
        {
            let _ = writeln!(f, "{line}");
        }
        println!("{line}");
        let _ = std::io::stdout().flush();
    };

    match flash(&device, &image, &provision_b64, no_verify, &append) {
        Ok(()) => {
            append("{\"stage\":\"done\"}");
            std::process::exit(0);
        }
        Err(e) => {
            let escaped = e.replace('\\', "\\\\").replace('"', "\\\"");
            append(&format!("{{\"error\":\"{escaped}\"}}"));
            std::process::exit(1);
        }
    }
}

/// Decode config, open device + image, run the engine. Returns the first error.
fn flash(
    device: &str,
    image: &str,
    provision_b64: &str,
    no_verify: bool,
    append: &dyn Fn(&str),
) -> Result<(), String> {
    let json = base64::engine::general_purpose::STANDARD
        .decode(provision_b64)
        .map_err(|e| format!("Failed to decode provision config: {e}"))?;
    let cfg: ProvisionConfig = serde_json::from_slice(&json)
        .map_err(|e| format!("Failed to parse provision config: {e}"))?;

    let mut image_file =
        std::fs::File::open(image).map_err(|e| format!("Failed to open image: {e}"))?;
    let image_len = image_file
        .metadata()
        .map(|m| m.len())
        .map_err(|e| format!("Failed to stat image: {e}"))?;

    let mut progress = |stage: FlashStage, done: u64, total: u64| {
        let stage = match stage {
            FlashStage::Write => "write",
            FlashStage::Verify => "verify",
            FlashStage::Customize => "customize",
        };
        append(&format!(
            "{{\"stage\":\"{stage}\",\"done\":{done},\"total\":{total}}}"
        ));
    };

    let params = FlashParams {
        image_len,
        verify: !no_verify,
    };

    // Open the raw device for this OS, wrap in AlignedDevice, run the engine,
    // then eject/flush. The device handle is held open the whole time.
    let result = open_and_run(device, &mut image_file, &params, &cfg, &mut progress);

    // Best-effort eject/finalize regardless of outcome.
    eject(device);

    result
}

/// Per-OS: open the raw device, wrap in AlignedDevice, drive the engine.
fn open_and_run(
    device: &str,
    image: &mut dyn std::io::Read,
    params: &FlashParams,
    cfg: &ProvisionConfig,
    progress: &mut dyn FnMut(FlashStage, u64, u64),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let raw = open_device_macos(device)?;
        let mut dev = AlignedDevice::new(raw);
        engine::flash(&mut dev, image, params, cfg, progress)
    }
    #[cfg(target_os = "linux")]
    {
        let raw = open_device_linux(device)?;
        let mut dev = AlignedDevice::new(raw);
        engine::flash(&mut dev, image, params, cfg, progress)
    }
    #[cfg(target_os = "windows")]
    {
        let raw = flasher_core::win::open_device(device)?;
        let mut dev = AlignedDevice::new(raw);
        engine::flash(&mut dev, image, params, cfg, progress)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (device, image, params, cfg, progress);
        Err("Flashing is not supported on this platform".to_string())
    }
}

// ---------------------------------------------------------------------------
// Unix `File` is a BlockDevice via fsync; provide sync over std::fs::File.
// ---------------------------------------------------------------------------

#[cfg(unix)]
struct UnixDevice(std::fs::File);

#[cfg(unix)]
impl std::io::Read for UnixDevice {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}
#[cfg(unix)]
impl std::io::Write for UnixDevice {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}
#[cfg(unix)]
impl std::io::Seek for UnixDevice {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        self.0.seek(pos)
    }
}
#[cfg(unix)]
impl engine::BlockDevice for UnixDevice {
    fn sync(&mut self) -> std::io::Result<()> {
        use std::os::unix::io::AsRawFd;
        // `File::sync_all()` issues `fcntl(F_FULLFSYNC)` on macOS, which a raw
        // character device (`/dev/rdiskN`) rejects with ENOTTY ("Inappropriate
        // ioctl for device"). Use a plain `fsync` like RPi Imager; and since the
        // device is opened with `F_NOCACHE`/`O_DIRECT` (already unbuffered),
        // tolerate ENOTTY as a successful no-op.
        let rc = unsafe { libc::fsync(self.0.as_raw_fd()) };
        if rc == 0 {
            return Ok(());
        }
        let err = std::io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::ENOTTY) {
            return Ok(());
        }
        Err(err)
    }
}

// ---------------------------------------------------------------------------
// macOS
// ---------------------------------------------------------------------------

/// Open `/dev/rdiskN` (the raw, unbuffered character device) with `F_NOCACHE`,
/// after force-unmounting the whole disk. The helper runs as root, so diskutil
/// and the open both succeed without prompting.
#[cfg(target_os = "macos")]
fn open_device_macos(device: &str) -> Result<UnixDevice, String> {
    use std::os::unix::io::FromRawFd;

    // /dev/diskN → /dev/rdiskN (character device: direct, unbuffered I/O).
    let raw_device = device.replace("/dev/disk", "/dev/rdisk");

    // Unmount the whole disk so the open succeeds and nothing dirties the FAT.
    let out = std::process::Command::new("/usr/sbin/diskutil")
        .args(["unmountDisk", "force", device])
        .output()
        .map_err(|e| format!("Failed to run diskutil unmountDisk: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "diskutil unmountDisk failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }

    let c_path = std::ffi::CString::new(raw_device.as_str())
        .map_err(|e| format!("Invalid device path: {e}"))?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        return Err(format!(
            "Failed to open {raw_device}: {}",
            std::io::Error::last_os_error()
        ));
    }
    // Disable the unified buffer cache so our writes go straight to the device.
    unsafe {
        if libc::fcntl(fd, libc::F_NOCACHE, 1) < 0 {
            // Non-fatal: AlignedDevice still keeps us sector-aligned.
        }
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    Ok(UnixDevice(file))
}

// ---------------------------------------------------------------------------
// Linux
// ---------------------------------------------------------------------------

/// Unmount any mounted partitions of the device, then open it `O_RDWR | O_DIRECT`
/// (falling back to plain `O_RDWR` if `O_DIRECT` is rejected).
#[cfg(target_os = "linux")]
fn open_device_linux(device: &str) -> Result<UnixDevice, String> {
    use std::os::unix::io::FromRawFd;

    // Unmount partitions like /dev/sdb1.. (best-effort; helper is root).
    for n in 1..=16 {
        let part = format!("{device}{n}");
        if std::path::Path::new(&part).exists() {
            let _ = std::process::Command::new("umount").arg(&part).output();
        }
    }

    let c_path = std::ffi::CString::new(device).map_err(|e| format!("Invalid device path: {e}"))?;

    // O_DIRECT bypasses the page cache (requires the AlignedDevice wrapper).
    let mut fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_DIRECT) };
    if fd < 0 {
        // Fall back to buffered O_RDWR if the FS/device rejects O_DIRECT.
        fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR) };
    }
    if fd < 0 {
        return Err(format!(
            "Failed to open {device}: {}",
            std::io::Error::last_os_error()
        ));
    }
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    Ok(UnixDevice(file))
}

// ---------------------------------------------------------------------------
// Eject / finalize
// ---------------------------------------------------------------------------

/// Best-effort eject after the engine returns, so the user can pull the card.
fn eject(device: &str) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("/usr/sbin/diskutil")
            .args(["eject", device])
            .output();
    }
    #[cfg(target_os = "linux")]
    {
        // Re-read the partition table so the kernel sees the new layout, then
        // eject. Both best-effort.
        let _ = std::process::Command::new("partprobe").arg(device).output();
        let _ = std::process::Command::new("eject").arg(device).output();
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        // Windows: the WinDevice Drop already unlocks/flushes; nothing to do.
        let _ = device;
    }
}
