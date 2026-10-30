use super::check_available_space;
use super::devices::list_block_devices;
use super::fits_on_card;
#[cfg(target_os = "windows")]
use super::hidden_command;
use super::private_temp_dir;
use super::randbits;
use super::usbboard;
use super::validate_disk_path;
use super::validate_fight_access;
use super::validate_image_file;
use super::Arc;
use super::AtomicBool;
use super::FlashPhase;
use super::FlashProgress;
use super::FlashSpeed;
use super::Ordering;
use super::ProvisionConfig;
use super::Read;
use super::HELPER_ABORT_GRACE;
use super::MIN_TEMP_SPACE_BYTES;
use super::PROGRESS_THROTTLE_MS;
use flasher_core::InitFormat;
use std::io::Write as _;

#[derive(Debug, Clone)]
pub enum FlashEvent {
    Progress(FlashProgress),
    Stage { stage: &'static str, bytes: u64 },
    Diag(String),
}

pub type FlashEvents = Arc<dyn Fn(FlashEvent) + Send + Sync>;

#[derive(Default)]
pub struct FlashTrace {
    pub image_bytes: u64,
    pub compressed: bool,
    pub decompress_ms: Option<u64>,
}

fn progress(
    events: &FlashEvents,
    phase: FlashPhase,
    done: u64,
    total: u64,
    percent: f64,
    bps: u64,
) {
    events(FlashEvent::Progress(FlashProgress {
        phase,
        bytes_processed: done,
        total_bytes: total,
        percent,
        speed_bps: bps,
    }));
}

pub async fn flash_image(
    config: &ProvisionConfig,
    image_path: &str,
    target_disk: &str,
    cancel: Arc<AtomicBool>,
    events: FlashEvents,
    trace: &mut FlashTrace,
) -> Result<(), String> {
    if usbboard::is_target(target_disk) {
        return flash_usb(config, image_path, target_disk, cancel, events, trace).await;
    }
    validate_disk_path(target_disk)?;
    validate_fight_access(config)?;
    let src_path = validate_image_file(std::path::Path::new(image_path))?;

    let devices = list_block_devices().await?;
    let Some(card) = devices.iter().find(|d| d.path == target_disk) else {
        return Err(format!(
            "Target disk {target_disk} is not a valid removable device"
        ));
    };
    let card_size = card.size;
    let card_label = format!("{} ({})", card.name, card.size_human);

    let temp_dir = private_temp_dir()?;

    let lower = src_path.to_string_lossy().to_ascii_lowercase();
    trace.compressed = lower.ends_with(".xz") || lower.ends_with(".gz");
    let (write_path, is_temp) = if trace.compressed {
        check_available_space(&temp_dir, MIN_TEMP_SPACE_BYTES)?;
        let dst = temp_dir.join("aircast-image.img");
        if dst.exists() {
            let _ = std::fs::remove_file(&dst);
        }
        (dst, true)
    } else {
        (src_path.clone(), false)
    };

    if !trace.compressed {
        let total = std::fs::metadata(&write_path).map(|m| m.len()).unwrap_or(0);
        fits_on_card(total, card_size, &card_label)?;
    }

    let provision_b64 = {
        use base64::Engine as _;
        let json = serde_json::to_vec(config)
            .map_err(|e| format!("Failed to encode provision config: {e}"))?;
        base64::engine::general_purpose::STANDARD.encode(json)
    };

    let ready_file = temp_dir.join(format!("aircast-image-ready-{}.txt", randbits()));
    let _ = std::fs::remove_file(&ready_file);

    let helper = tokio::spawn(run_elevated_flash(
        target_disk.to_string(),
        write_path.clone(),
        ready_file.clone(),
        provision_b64,
        events.clone(),
        cancel.clone(),
    ));

    let prepared = if trace.compressed {
        let unpack_started = std::time::Instant::now();
        decompress(&events, &src_path, &write_path, &lower, cancel.clone())
            .await
            .map(|_| {
                trace.decompress_ms = Some(unpack_started.elapsed().as_millis() as u64);
            })
    } else {
        Ok(())
    };

    let prepared = prepared.and_then(|()| {
        let total = std::fs::metadata(&write_path).map(|m| m.len()).unwrap_or(0);
        trace.image_bytes = total;
        fits_on_card(total, card_size, &card_label).map(|()| total)
    });

    let result = match prepared {
        Ok(total) => {
            let _ = std::fs::write(&ready_file, "ok");
            progress(&events, FlashPhase::Writing, 0, total, 0.0, 0);
            helper
                .await
                .unwrap_or_else(|e| Err(format!("Elevated flash task failed: {e}")))
        }
        Err(prep_error) => {
            let _ = std::fs::write(&ready_file, "abort");
            let helper_result = match tokio::time::timeout(HELPER_ABORT_GRACE, helper).await {
                Ok(joined) => {
                    joined.unwrap_or_else(|e| Err(format!("Elevated flash task failed: {e}")))
                }
                Err(_) => Ok(()),
            };
            match helper_result {
                Err(e) if !e.contains("image preparation aborted") => Err(e),
                _ => Err(prep_error),
            }
        }
    };

    let _ = std::fs::remove_file(&ready_file);
    if is_temp {
        let _ = std::fs::remove_file(&write_path);
    }
    result
}

async fn flash_usb(
    config: &ProvisionConfig,
    image_path: &str,
    target: &str,
    cancel: Arc<AtomicBool>,
    events: FlashEvents,
    trace: &mut FlashTrace,
) -> Result<(), String> {
    validate_fight_access(config)?;
    let family = usbboard::family_of_target(target)
        .ok_or_else(|| format!("{target} is not a board this Flasher can write over USB"))?;
    let src_path = validate_image_file(std::path::Path::new(image_path))?;
    let lower = src_path.to_string_lossy().to_ascii_lowercase();
    trace.compressed = lower.ends_with(".xz") || lower.ends_with(".gz");
    let temp_dir = private_temp_dir()?;
    check_available_space(&temp_dir, MIN_TEMP_SPACE_BYTES)?;
    let write_path = temp_dir.join("aircast-image.img");
    let _ = std::fs::remove_file(&write_path);
    let result = async {
        if trace.compressed {
            let started = std::time::Instant::now();
            decompress(&events, &src_path, &write_path, &lower, cancel.clone()).await?;
            trace.decompress_ms = Some(started.elapsed().as_millis() as u64);
        } else {
            std::fs::copy(&src_path, &write_path)
                .map_err(|e| format!("Failed to copy the image: {e}"))?;
        }
        events(FlashEvent::Stage {
            stage: "customize",
            bytes: 0,
        });
        customize_file(&write_path, config).await?;
        write_usb(family, &write_path, cancel, events, trace).await
    }
    .await;
    let _ = std::fs::remove_file(&write_path);
    result
}

async fn customize_file(image: &std::path::Path, config: &ProvisionConfig) -> Result<(), String> {
    let image = image.to_path_buf();
    let config = config.clone();
    tokio::task::spawn_blocking(move || {
        let mut file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&image)
            .map_err(|e| format!("Failed to open the image: {e}"))?;
        flasher_core::customize_image(&mut file, &config)
    })
    .await
    .map_err(|e| format!("The customize task failed: {e}"))?
}

async fn write_usb(
    family: &'static usbboard::Family,
    image: &std::path::Path,
    cancel: Arc<AtomicBool>,
    events: FlashEvents,
    trace: &mut FlashTrace,
) -> Result<(), String> {
    let image_len = std::fs::metadata(image)
        .map(|m| m.len())
        .map_err(|e| format!("Failed to read the image: {e}"))?;
    trace.image_bytes = image_len;
    events(FlashEvent::Stage {
        stage: "device",
        bytes: 0,
    });
    let disk = usbboard::open(family, cancel.clone()).await?;
    fits_on_card(
        image_len,
        disk.size(),
        &format!("the board's built-in storage ({})", disk.size_human()),
    )?;
    let image = image.to_path_buf();
    let task_events = events.clone();
    let written = tokio::task::spawn_blocking(move || {
        let mut disk = disk;
        let customized = ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: None,
            access: None,
            encryption: None,
            fight_mode: false,
            init_format: InitFormat::CloudInit,
        };
        let mut file =
            std::fs::File::open(&image).map_err(|e| format!("Failed to open the image: {e}"))?;
        let mut speed = FlashSpeed::default();
        flasher_core::flash(
            &mut disk,
            &mut file,
            &flasher_core::FlashParams {
                image_len,
                verify: false,
            },
            &customized,
            &mut |stage, done, total| {
                let phase = match stage {
                    flasher_core::FlashStage::Write => FlashPhase::Writing,
                    flasher_core::FlashStage::Verify => FlashPhase::Verifying,
                    flasher_core::FlashStage::Customize => FlashPhase::Customizing,
                };
                let percent = if total > 0 {
                    done as f64 / total as f64 * 100.0
                } else {
                    0.0
                };
                progress(
                    &task_events,
                    phase,
                    done,
                    total,
                    percent,
                    speed.sample(phase, done),
                );
            },
        )
    })
    .await
    .map_err(|e| format!("The USB write task failed: {e}"))?;
    match written {
        Err(_) if cancel.load(Ordering::SeqCst) => Err("Cancelled".to_string()),
        Err(e) => Err(e),
        Ok(()) => usbboard::restart(family).await,
    }
}

fn stage_label(stage: &str) -> Option<&'static str> {
    match stage {
        "device" => Some("device"),
        "write" => Some("write"),
        "verify" => Some("verify"),
        "customize" => Some("customize"),
        _ => None,
    }
}

fn phase_from_stage(stage: &str) -> Option<FlashPhase> {
    match stage {
        "write" => Some(FlashPhase::Writing),
        "verify" => Some(FlashPhase::Verifying),
        "customize" => Some(FlashPhase::Customizing),
        _ => None,
    }
}

async fn run_elevated_flash(
    target_disk: String,
    image: std::path::PathBuf,
    ready_file: std::path::PathBuf,
    provision_b64: String,
    events: FlashEvents,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Failed to find current exe: {e}"))?;

    let progress_file = std::env::temp_dir().join(format!(
        "aircast-flash-progress-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&progress_file);
    std::fs::File::create(&progress_file)
        .map_err(|e| format!("Failed to create progress file: {e}"))?;

    let tail_events = events.clone();
    let pf_for_tail = progress_file.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_tail = stop.clone();
    let tail = tokio::spawn(async move {
        let mut offset: u64 = 0;
        let mut speed = FlashSpeed::default();
        loop {
            if let Ok(bytes) = std::fs::read(&pf_for_tail) {
                if (bytes.len() as u64) > offset {
                    let new = String::from_utf8_lossy(&bytes[offset as usize..]).to_string();
                    offset = bytes.len() as u64;
                    for line in new.lines() {
                        let line = line.trim();
                        if line.is_empty() {
                            continue;
                        }
                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                            if let Some(d) = v.get("diag").and_then(|x| x.as_str()) {
                                tail_events(FlashEvent::Diag(d.to_string()));
                                continue;
                            }
                            let stage = v.get("stage").and_then(|x| x.as_str());
                            let done = v.get("done").and_then(|x| x.as_u64()).unwrap_or(0);
                            let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0);
                            if let Some(stage) = stage {
                                if let Some(label) = stage_label(stage) {
                                    tail_events(FlashEvent::Stage {
                                        stage: label,
                                        bytes: done,
                                    });
                                }
                                if let Some(phase) = phase_from_stage(stage) {
                                    let percent = if total > 0 {
                                        (done as f64 / total as f64) * 100.0
                                    } else {
                                        0.0
                                    };
                                    let bps = speed.sample(phase, done);
                                    progress(&tail_events, phase, done, total, percent, bps);
                                }
                            }
                        }
                    }
                }
            }
            if stop_tail.load(Ordering::SeqCst) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(
                PROGRESS_THROTTLE_MS as u64,
            ))
            .await;
        }
    });

    sweep_stale_provision_files();
    let provision_file =
        std::env::temp_dir().join(format!("{PROVISION_FILE_PREFIX}{}.b64", randbits()));
    write_secret_file(&provision_file, &provision_b64)?;
    let provision_arg = provision_file.to_string_lossy().to_string();

    let launch = launch_helper(
        &exe,
        &target_disk,
        &image,
        &progress_file,
        &provision_arg,
        &ready_file,
    )
    .await;
    let _ = std::fs::remove_file(&provision_file);

    stop.store(true, Ordering::SeqCst);
    let _ = tail.await;

    let contents = std::fs::read_to_string(&progress_file).unwrap_or_default();
    let last_error = contents
        .lines()
        .rev()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l.trim()).ok())
        .find_map(|v| {
            v.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        });
    let _ = std::fs::remove_file(&progress_file);

    let out = match launch {
        Ok(true) => Ok(()),
        Ok(false) => Err(last_error.unwrap_or_else(|| {
            "Elevated flash failed (authorization declined or write error)".to_string()
        })),
        Err(e) => Err(last_error.unwrap_or(e)),
    };
    if out.is_err() {
        cancel.store(true, Ordering::SeqCst);
    }
    out
}

const PROVISION_FILE_PREFIX: &str = "aircast-provision-";

fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write as _;
    let _ = std::fs::remove_file(path);
    let mut f = {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
        }
        #[cfg(not(unix))]
        {
            std::fs::File::create(path)
        }
    }
    .map_err(|e| format!("Failed to create provision file: {e}"))?;
    f.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write provision file: {e}"))?;
    Ok(())
}

const STALE_PROVISION_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

fn sweep_stale_provision_files() {
    sweep_provision_files_in(&std::env::temp_dir(), STALE_PROVISION_AGE);
}

fn sweep_provision_files_in(dir: &std::path::Path, max_age: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with(PROVISION_FILE_PREFIX)
        {
            continue;
        }
        let too_old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age >= max_age);
        if too_old {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

#[cfg(unix)]
fn helper_args(
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_file: &str,
    ready_file: &std::path::Path,
) -> Vec<String> {
    [
        "--flash-helper",
        "--device",
        target_disk,
        "--image",
        &image.to_string_lossy(),
        "--progress-file",
        &progress_file.to_string_lossy(),
        "--provision-file",
        provision_file,
        "--image-ready-file",
        &ready_file.to_string_lossy(),
    ]
    .iter()
    .map(|arg| arg.to_string())
    .collect()
}

#[cfg(unix)]
fn running_as_root() -> bool {
    unsafe { libc::geteuid() == 0 }
}

#[cfg(unix)]
async fn run_helper_directly(exe: &std::path::Path, args: &[String]) -> Result<bool, String> {
    tokio::process::Command::new(exe)
        .args(args)
        .status()
        .await
        .map(|status| status.success())
        .map_err(|e| format!("Failed to launch the flash helper: {e}"))
}

#[cfg(target_os = "macos")]
async fn launch_helper(
    exe: &std::path::Path,
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_file: &str,
    ready_file: &std::path::Path,
) -> Result<bool, String> {
    use crate::macos_auth::Authorization;

    let args = helper_args(
        target_disk,
        image,
        progress_file,
        provision_file,
        ready_file,
    );
    if running_as_root() {
        return run_helper_directly(exe, &args).await;
    }

    let exe = exe.to_string_lossy().to_string();
    tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let auth = Authorization::new()?;
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        match auth.execute(&exe, &args) {
            Ok(_) => Ok(true),
            Err(e) if e.contains("Authentication cancelled") => Err(e),
            Err(_) => Ok(false),
        }
    })
    .await
    .map_err(|e| format!("Elevated flash task failed: {e}"))?
}

#[cfg(target_os = "linux")]
async fn launch_helper(
    exe: &std::path::Path,
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_file: &str,
    ready_file: &std::path::Path,
) -> Result<bool, String> {
    let args = helper_args(
        target_disk,
        image,
        progress_file,
        provision_file,
        ready_file,
    );
    if running_as_root() {
        return run_helper_directly(exe, &args).await;
    }
    let status = tokio::process::Command::new("pkexec")
        .arg(exe)
        .args(&args)
        .status()
        .await
        .map_err(|e| format!("Failed to launch elevated flasher via pkexec: {e}"))?;
    Ok(status.success())
}

#[cfg(target_os = "windows")]
async fn launch_helper(
    exe: &std::path::Path,
    target_disk: &str,
    image: &std::path::Path,
    progress_file: &std::path::Path,
    provision_file: &str,
    ready_file: &std::path::Path,
) -> Result<bool, String> {
    let ps_cmd = format!(
        "$p = Start-Process -FilePath '{exe}' -ArgumentList \
         '--flash-helper','--device','{device}','--image','{image}','--progress-file','{pf}','--provision-file','{provision}','--image-ready-file','{ready}' \
         -Verb RunAs -Wait -PassThru; exit $p.ExitCode",
        exe = ps_escape(&exe.to_string_lossy()),
        device = ps_escape(target_disk),
        image = ps_escape(&image.to_string_lossy()),
        pf = ps_escape(&progress_file.to_string_lossy()),
        provision = ps_escape(provision_file),
        ready = ps_escape(&ready_file.to_string_lossy()),
    );

    let status = hidden_command("powershell")
        .args(["-NoProfile", "-Command", &ps_cmd])
        .status()
        .await
        .map_err(|e| format!("Failed to launch elevated flasher: {e}"))?;
    Ok(status.success())
}

#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
async fn launch_helper(
    _exe: &std::path::Path,
    _target_disk: &str,
    _image: &std::path::Path,
    _progress_file: &std::path::Path,
    _provision_b64: &str,
    _ready_file: &std::path::Path,
) -> Result<bool, String> {
    Err("Disk flashing is not supported on this platform".to_string())
}

#[cfg(target_os = "windows")]
fn ps_escape(s: &str) -> String {
    s.replace('\'', "''")
}

async fn decompress(
    events: &FlashEvents,
    src: &std::path::Path,
    dst: &std::path::Path,
    lower_name: &str,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    progress(events, FlashPhase::Decompressing, 0, 0, 0.0, 0);

    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    let events = events.clone();
    let is_gz = lower_name.ends_with(".gz");

    tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&src).map_err(|e| format!("Failed to open image: {e}"))?;
        let mut decoder: Box<dyn Read> = if is_gz {
            Box::new(flate2::read::GzDecoder::new(file))
        } else {
            Box::new(xz2::read::XzDecoder::new(file))
        };
        let mut output = std::fs::File::create(&dst)
            .map_err(|e| format!("Failed to create decompressed file: {e}"))?;

        let mut buffer = vec![0u8; 1024 * 1024];
        let mut bytes_written: u64 = 0;
        let mut last_emit = std::time::Instant::now();
        loop {
            if cancel.load(Ordering::SeqCst) {
                let _ = std::fs::remove_file(&dst);
                return Err("Flash cancelled".to_string());
            }
            let n = decoder
                .read(&mut buffer)
                .map_err(|e| format!("Decompression error: {e}"))?;
            if n == 0 {
                break;
            }
            output
                .write_all(&buffer[..n])
                .map_err(|e| format!("Write error: {e}"))?;
            bytes_written += n as u64;
            if last_emit.elapsed().as_millis() >= PROGRESS_THROTTLE_MS {
                progress(&events, FlashPhase::Decompressing, bytes_written, 0, 0.0, 0);
                last_emit = std::time::Instant::now();
            }
        }
        progress(
            &events,
            FlashPhase::Decompressing,
            bytes_written,
            bytes_written,
            100.0,
            0,
        );
        Ok(bytes_written)
    })
    .await
    .map_err(|e| format!("Decompress task failed: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolated_dir;
    use crate::MIN_CARD_BYTES;

    #[test]
    fn card_check_uses_the_decompressed_size() {
        const UNCOMPRESSED: u64 = 2_969_567_232;
        const COMPRESSED: u64 = 586_187_284;
        const ONE_GB_CARD: u64 = 1_000_000_000;

        const _: () = assert!(MIN_CARD_BYTES > UNCOMPRESSED);
        const _: () = assert!(COMPRESSED < ONE_GB_CARD && ONE_GB_CARD < MIN_CARD_BYTES);

        assert!(
            fits_on_card(UNCOMPRESSED, ONE_GB_CARD, "SD (1 GB)").is_err(),
            "a 1 GB card cannot hold a 2.97 GB image"
        );
        assert!(
            fits_on_card(UNCOMPRESSED, 4_000_000_000, "SD (4 GB)").is_ok(),
            "a real 4 GB card must not be rejected"
        );
        assert!(
            fits_on_card(UNCOMPRESSED, 0, "SD (unknown)").is_ok(),
            "an unknown card size must not block the flash"
        );
    }

    #[test]
    fn write_secret_file_round_trips_contents() {
        let dir = isolated_dir("roundtrip");
        let path = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&path, "secret-blob").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "secret-blob");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn write_secret_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = isolated_dir("mode");
        let path = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&path, "secret-blob").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "provision file must be 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_removes_stale_provision_files() {
        let dir = isolated_dir("sweep-stale");
        let stale = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&stale, "x").unwrap();
        sweep_provision_files_in(&dir, std::time::Duration::ZERO);
        assert!(
            !stale.exists(),
            "sweep must remove orphaned provision files"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_keeps_recent_provision_files() {
        let dir = isolated_dir("sweep-recent");
        let fresh = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&fresh, "x").unwrap();
        sweep_provision_files_in(&dir, std::time::Duration::from_secs(3600));
        assert!(
            fresh.exists(),
            "sweep must not delete an in-flight provision file"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flash_phase_serializes_snake_case() {
        assert_eq!(
            serde_json::to_string(&FlashPhase::Writing).unwrap(),
            "\"writing\""
        );
        assert_eq!(
            serde_json::to_string(&FlashPhase::Decompressing).unwrap(),
            "\"decompressing\""
        );
    }
}
