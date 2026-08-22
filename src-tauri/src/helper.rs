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
    let provision_file = get("--provision-file");
    let ready_file = get("--image-ready-file");
    let no_verify = args.iter().any(|a| a == "--no-verify");

    let (device, image, progress_file, provision_file) =
        match (device, image, progress_file, provision_file) {
            (Some(d), Some(i), Some(p), Some(c)) => (d, i, p, c),
            _ => {
                eprintln!(
                    "flash-helper: missing --device/--image/--progress-file/--provision-file"
                );
                std::process::exit(2);
            }
        };

    // Append a single JSON line to the progress file AND stdout (best-effort).
    let started = std::time::Instant::now();
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
    let diag = {
        let append = append.clone();
        move |msg: &str| {
            let line = serde_json::json!({
                "diag": format!("+{}ms {msg}", started.elapsed().as_millis()),
            });
            append(&line.to_string());
        }
    };

    // The provision blob carries secrets (device password, Tailscale key) so it
    // is passed by file path, not on the command line where `ps` could read it.
    // Read it once, then remove the file so it doesn't linger.
    let provision_b64 = match std::fs::read_to_string(&provision_file) {
        Ok(s) => {
            let _ = std::fs::remove_file(&provision_file);
            s.trim().to_string()
        }
        Err(e) => {
            let escaped = e.to_string().replace('\\', "\\\\").replace('"', "\\\"");
            append(&format!(
                "{{\"error\":\"Failed to read provision file: {escaped}\"}}"
            ));
            std::process::exit(1);
        }
    };

    match flash(
        &device,
        &image,
        ready_file.as_deref(),
        &provision_b64,
        no_verify,
        &append,
        &diag,
    ) {
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

/// Decode config, open + probe the device FIRST (fail fast, before the main
/// process finishes decompressing), then wait for the image and run the engine.
fn flash(
    device: &str,
    image: &str,
    ready_file: Option<&str>,
    provision_b64: &str,
    no_verify: bool,
    append: &dyn Fn(&str),
    diag: &dyn Fn(&str),
) -> Result<(), String> {
    let json = base64::engine::general_purpose::STANDARD
        .decode(provision_b64)
        .map_err(|e| format!("Failed to decode provision config: {e}"))?;
    let cfg: ProvisionConfig = serde_json::from_slice(&json)
        .map_err(|e| format!("Failed to parse provision config: {e}"))?;

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

    // Open the raw device for this OS, wrap in AlignedDevice, run the engine,
    // then eject/flush. The device handle is held open the whole time.
    append("{\"stage\":\"device\",\"done\":0,\"total\":0}");
    diag(&format!("opening device {device}"));
    let result = open_and_run(
        device, image, ready_file, no_verify, &cfg, &mut progress, diag,
    );

    // Best-effort eject/finalize regardless of outcome.
    eject(device);

    result
}

/// Per-OS: open the raw device, wrap in AlignedDevice, drive the flash.
fn open_and_run(
    device: &str,
    image: &str,
    ready_file: Option<&str>,
    no_verify: bool,
    cfg: &ProvisionConfig,
    progress: &mut dyn FnMut(FlashStage, u64, u64),
    diag: &dyn Fn(&str),
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let raw = open_device_macos(device)?;
        let mut dev = AlignedDevice::new(raw);
        probe_wait_and_flash(&mut dev, image, ready_file, no_verify, cfg, progress, diag)
    }
    #[cfg(target_os = "linux")]
    {
        let raw = open_device_linux(device)?;
        let mut dev = AlignedDevice::new(raw);
        probe_wait_and_flash(&mut dev, image, ready_file, no_verify, cfg, progress, diag)
    }
    #[cfg(target_os = "windows")]
    {
        let raw = flasher_core::win::open_device(device)?;
        let mut dev = AlignedDevice::new(raw);
        probe_wait_and_flash(&mut dev, image, ready_file, no_verify, cfg, progress, diag)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = (device, image, ready_file, no_verify, cfg, progress, diag);
        Err("Flashing is not supported on this platform".to_string())
    }
}

/// Give up waiting for the decompressed image after this long. A ~3 GB xz
/// unpacks in well under a minute on any machine that can run this app, so
/// overshooting it means the main process died — and while we wait, a root
/// process is holding the card's raw device open. Minutes, not half an hour.
const IMAGE_WAIT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5 * 60);

/// How often [`wait_for_image_ready`] re-reads the handshake file.
const IMAGE_WAIT_POLL: std::time::Duration = std::time::Duration::from_millis(200);

/// Probe the opened device (seek to its end and back — the exact operation that
/// used to fail 50 seconds in), wait for the decompressed image if a ready-file
/// handshake was requested, then run the engine.
fn probe_wait_and_flash(
    dev: &mut dyn engine::BlockDevice,
    image: &str,
    ready_file: Option<&str>,
    no_verify: bool,
    cfg: &ProvisionConfig,
    progress: &mut dyn FnMut(FlashStage, u64, u64),
    diag: &dyn Fn(&str),
) -> Result<(), String> {
    use std::io::SeekFrom;

    let size = dev
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("device probe: seek to end of device: {e}"))?;
    dev.seek(SeekFrom::Start(0))
        .map_err(|e| format!("device probe: seek to start of device: {e}"))?;
    diag(&format!("device open, size {size} bytes"));

    if let Some(rf) = ready_file {
        diag("waiting for image (decompressing in main process)");
        wait_for_image_ready(rf, IMAGE_WAIT_TIMEOUT)?;
        diag("image ready");
    }

    let mut image_file =
        std::fs::File::open(image).map_err(|e| format!("Failed to open image: {e}"))?;
    let image_len = image_file
        .metadata()
        .map(|m| m.len())
        .map_err(|e| format!("Failed to stat image: {e}"))?;
    diag(&format!("image open, {image_len} bytes"));

    let params = FlashParams {
        image_len,
        verify: !no_verify,
    };
    engine::flash(dev, &mut image_file, &params, cfg, progress)
}

/// Poll for the ready file the main process writes after decompression:
/// contents "ok" → proceed, anything else → the decompress failed or was
/// cancelled, so exit without touching the card further.
fn wait_for_image_ready(ready_file: &str, timeout: std::time::Duration) -> Result<(), String> {
    let started = std::time::Instant::now();
    loop {
        if let Ok(s) = std::fs::read_to_string(ready_file) {
            let s = s.trim();
            if !s.is_empty() {
                return if s == "ok" {
                    Ok(())
                } else {
                    Err("Cancelled before writing (image preparation aborted)".to_string())
                };
            }
        }
        if started.elapsed() > timeout {
            return Err("Timed out waiting for the decompressed image".to_string());
        }
        std::thread::sleep(IMAGE_WAIT_POLL.min(timeout));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "aircast-helper-test-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("ready.txt")
    }

    #[test]
    fn ready_file_ok_lets_the_write_start() {
        let p = temp_path("ok");
        std::fs::write(&p, "ok\n").unwrap();
        assert!(wait_for_image_ready(p.to_str().unwrap(), IMAGE_WAIT_TIMEOUT).is_ok());
    }

    #[test]
    fn ready_file_abort_stops_the_write() {
        let p = temp_path("abort");
        std::fs::write(&p, "abort").unwrap();
        let err = wait_for_image_ready(p.to_str().unwrap(), IMAGE_WAIT_TIMEOUT)
            .expect_err("abort must not proceed to writing");
        assert!(err.contains("image preparation aborted"), "got: {err}");
    }

    /// A missing or still-empty file means the main process is mid-decompress:
    /// wait, then give up rather than hold the card's device open forever.
    #[test]
    fn missing_ready_file_times_out_instead_of_writing() {
        let p = temp_path("timeout");
        let err = wait_for_image_ready(p.to_str().unwrap(), std::time::Duration::ZERO)
            .expect_err("a main process that never signals must not start a write");
        assert!(err.contains("Timed out"), "got: {err}");
    }
}
