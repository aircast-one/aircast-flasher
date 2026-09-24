use super::check_available_space;
use super::devices::list_block_devices;
use super::emit_flash_progress;
use super::fits_on_card;
#[cfg(target_os = "windows")]
use super::hidden_command;
use super::private_temp_dir;
use super::randbits;
use super::validate_disk_path;
use super::validate_image_file;
use super::AppHandle;
use super::Arc;
use super::AtomicBool;
use super::FlashPhase;
use super::FlashSpeed;
use super::FlasherState;
use super::Ordering;
use super::ProvisionConfig;
use super::Read;
use super::TailscaleConfig;
use super::WifiConfig;
use super::HELPER_ABORT_GRACE;
use super::MIN_TEMP_SPACE_BYTES;
use super::PROGRESS_THROTTLE_MS;
use crate::telemetry;
use std::io::Write as _;

// ---------------------------------------------------------------------------
// Command: flash_image (decompress if needed, then dd)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct FlashTrace {
    image_bytes: u64,
    compressed: bool,
    decompress_ms: Option<u64>,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn flash_image(
    app_handle: AppHandle,
    state: tauri::State<'_, FlasherState>,
    job_id: String,
    image_path: String,
    target_disk: String,
    wifi: Option<WifiConfig>,
    hostname: Option<String>,
    tailscale: Option<TailscaleConfig>,
    access: Option<flasher_core::AccessConfig>,
    init_format: flasher_core::InitFormat,
) -> Result<(), String> {
    let config = ProvisionConfig {
        hostname,
        wifi,
        tailscale,
        access,
        init_format,
    };
    let facts = telemetry::describe_config(&config);
    let phases = Arc::new(std::sync::Mutex::new(telemetry::PhaseLog::default()));
    let cancelled = state.user_cancel.clone();
    let started = std::time::Instant::now();
    let mut trace = FlashTrace::default();

    let result = flash_image_inner(
        &app_handle,
        &state,
        &config,
        image_path,
        target_disk,
        phases.clone(),
        &mut trace,
    )
    .await;

    let end = std::time::Instant::now();
    let duration_ms = started.elapsed().as_millis() as u64;
    let log = phases.lock().expect("phase log poisoned");
    let write_ms = log.duration_ms("write", end);
    let outcome = match &result {
        Ok(()) => "success",
        Err(_) if cancelled.load(Ordering::SeqCst) => "cancelled",
        Err(_) => "failed",
    };
    let failed_at = result.as_ref().err().map(|_| {
        log.last_stage()
            .unwrap_or(if trace.decompress_ms.is_some() {
                "launch"
            } else if trace.compressed {
                "decompress"
            } else {
                "prepare"
            })
    });

    telemetry::record(
        &app_handle,
        &telemetry::FlashEvent {
            envelope: telemetry::Envelope::new(
                "flash",
                job_id,
                &app_handle,
                outcome,
                duration_ms,
                result.as_ref().err().cloned(),
            ),
            config: facts,
            image_bytes: trace.image_bytes,
            compressed: trace.compressed,
            verified: log.saw("verify"),
            failed_at,
            decompress_ms: trace.decompress_ms,
            write_ms,
            verify_ms: log.duration_ms("verify", end),
            customize_ms: log.duration_ms("customize", end),
            write_mbps: write_ms.and_then(|ms| telemetry::mbps(trace.image_bytes, ms)),
            diag: (result.is_err() && !log.diag.is_empty()).then(|| log.diag.clone()),
        },
    );

    result
}

async fn flash_image_inner(
    app_handle: &AppHandle,
    state: &tauri::State<'_, FlasherState>,
    config: &ProvisionConfig,
    image_path: String,
    target_disk: String,
    phases: Arc<std::sync::Mutex<telemetry::PhaseLog>>,
    trace: &mut FlashTrace,
) -> Result<(), String> {
    validate_disk_path(&target_disk)?;
    let src_path = validate_image_file(std::path::Path::new(&image_path))?;

    // Re-validate the target is still a removable external device.
    let devices = list_block_devices().await?;
    let Some(card) = devices.iter().find(|d| d.path == target_disk) else {
        return Err(format!(
            "Target disk {target_disk} is not a valid removable device"
        ));
    };
    let card_size = card.size;
    let card_label = format!("{} ({})", card.name, card.size_human);

    state.user_cancel.store(false, Ordering::SeqCst);
    state.cancel.store(false, Ordering::SeqCst);
    let cancel = state.cancel.clone();

    let temp_dir = private_temp_dir()?;

    // Decompress only if compressed; raw .img is written directly (local files).
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

    // For an uncompressed image the size is known now, so a card that cannot
    // hold it fails before the elevation prompt instead of after it.
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

    // Launch the elevated helper FIRST so it opens and probes the device while
    // the image decompresses: a bad device/reader fails in seconds, not after a
    // 50-second decompress. The ready-file handshake tells the helper when the
    // image is complete ("ok") or the preparation died ("abort"). On failure the
    // helper sets `cancel`, which aborts the concurrent decompress.
    let ready_file = temp_dir.join(format!("aircast-image-ready-{}.txt", randbits()));
    let _ = std::fs::remove_file(&ready_file);

    let helper = tokio::spawn(run_elevated_flash(
        app_handle.clone(),
        target_disk.clone(),
        write_path.clone(),
        ready_file.clone(),
        provision_b64,
        phases,
        cancel.clone(),
    ));

    let prepared = if trace.compressed {
        let unpack_started = std::time::Instant::now();
        decompress(app_handle, &src_path, &write_path, &lower, cancel.clone())
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
            emit_flash_progress(app_handle, FlashPhase::Writing, 0, total, 0.0, 0);
            helper
                .await
                .unwrap_or_else(|e| Err(format!("Elevated flash task failed: {e}")))
        }
        Err(prep_error) => {
            let _ = std::fs::write(&ready_file, "abort");
            // The helper may still be sitting behind an unanswered Touch ID/UAC
            // prompt, where it cannot see the abort file at all. Report the real
            // failure rather than blocking on the user dismissing a dialog for a
            // flash that has already lost.
            let helper_result = match tokio::time::timeout(HELPER_ABORT_GRACE, helper).await {
                Ok(joined) => {
                    joined.unwrap_or_else(|e| Err(format!("Elevated flash task failed: {e}")))
                }
                Err(_) => Ok(()),
            };
            // If the helper died on its own (device error, declined elevation),
            // that is the root cause and the decompress abort is collateral.
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

/// The helper's stage strings, as `'static` labels for the phase log.
fn stage_label(stage: &str) -> Option<&'static str> {
    match stage {
        "device" => Some("device"),
        "write" => Some("write"),
        "verify" => Some("verify"),
        "customize" => Some("customize"),
        _ => None,
    }
}

/// Map a helper progress `stage` string to a [`FlashPhase`].
fn phase_from_stage(stage: &str) -> Option<FlashPhase> {
    match stage {
        "write" => Some(FlashPhase::Writing),
        "verify" => Some(FlashPhase::Verifying),
        "customize" => Some(FlashPhase::Customizing),
        _ => None,
    }
}

/// Launch the elevated helper (this same binary `--flash-helper`) as
/// root/admin, while a tailer thread reads the progress file it appends to and
/// re-emits `flasher:flash-progress`. The elevation mechanism is per-OS.
async fn run_elevated_flash(
    app_handle: AppHandle,
    target_disk: String,
    image: std::path::PathBuf,
    ready_file: std::path::PathBuf,
    provision_b64: String,
    phases: Arc<std::sync::Mutex<telemetry::PhaseLog>>,
    cancel: Arc<AtomicBool>,
) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("Failed to find current exe: {e}"))?;

    // A unique progress file in the temp dir, shared with the elevated helper.
    let progress_file = std::env::temp_dir().join(format!(
        "aircast-flash-progress-{}.jsonl",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&progress_file);
    std::fs::File::create(&progress_file)
        .map_err(|e| format!("Failed to create progress file: {e}"))?;

    // Tailer: poll the progress file and emit flash progress until told to stop.
    let app = app_handle.clone();
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
                                if let Ok(mut log) = phases.lock() {
                                    log.diag.push(d.to_string());
                                }
                                continue;
                            }
                            let stage = v.get("stage").and_then(|x| x.as_str());
                            let done = v.get("done").and_then(|x| x.as_u64()).unwrap_or(0);
                            let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0);
                            if let Some(stage) = stage {
                                if let (Some(label), Ok(mut log)) =
                                    (stage_label(stage), phases.lock())
                                {
                                    log.mark(label, done);
                                }
                                if let Some(phase) = phase_from_stage(stage) {
                                    let percent = if total > 0 {
                                        (done as f64 / total as f64) * 100.0
                                    } else {
                                        0.0
                                    };
                                    let bps = speed.sample(phase, done);
                                    emit_flash_progress(&app, phase, done, total, percent, bps);
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

    // The provision blob holds secrets (device password, Tailscale key), so it
    // goes to the helper as a 0600 file path — never on the command line, where
    // any local user could read it via `ps`.
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

    // Stop the tailer regardless of outcome.
    stop.store(true, Ordering::SeqCst);
    let _ = tail.await;

    // Re-read the progress file for a terminal error line.
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
    // A helper that died while the main process is still decompressing must
    // abort that decompress: its cancel check is the only thing watching us.
    if out.is_err() {
        cancel.store(true, Ordering::SeqCst);
    }
    out
}

const PROVISION_FILE_PREFIX: &str = "aircast-provision-";

/// Writes `contents` to `path` as a `0600` file (owner-only on Unix), replacing
/// any existing file. Used for the secret-bearing provision blob.
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
        // Windows has no mode bits here; the file lands in the user's temp dir
        // (readable by the same user and admins). The argv exposure is what
        // mattered — a locked-down ACL would need the windows-acl crate.
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

/// No live flash runs anywhere near this long, so a provision blob older than
/// this is an orphan from a crashed flasher.
const STALE_PROVISION_AGE: std::time::Duration = std::time::Duration::from_secs(3600);

/// Best-effort removal of orphaned provision blobs in the temp dir, left by a
/// flasher that crashed between writing the file and cleaning it up. Only files
/// older than [`STALE_PROVISION_AGE`] are removed, so a concurrently-running
/// flasher's in-flight blob is never deleted.
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

/// Per-OS elevation launcher. Returns `Ok(true)` if the helper exited 0,
/// `Ok(false)` on a non-zero exit, `Err` if the launcher itself failed.
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

    let exe = exe.to_string_lossy().to_string();
    let device = target_disk.to_string();
    let image = image.to_string_lossy().to_string();
    let pf = progress_file.to_string_lossy().to_string();
    let provision = provision_file.to_string();
    let ready = ready_file.to_string_lossy().to_string();

    // One Touch ID prompt; the helper inherits root and holds the device open.
    tokio::task::spawn_blocking(move || -> Result<bool, String> {
        let auth = Authorization::new()?;
        match auth.execute(
            &exe,
            &[
                "--flash-helper",
                "--device",
                &device,
                "--image",
                &image,
                "--progress-file",
                &pf,
                "--provision-file",
                &provision,
                "--image-ready-file",
                &ready,
            ],
        ) {
            Ok(_) => Ok(true),
            Err(e) => {
                if e.contains("Authentication cancelled") {
                    Err(e)
                } else {
                    // Non-zero exit from the helper → surface the progress error.
                    Ok(false)
                }
            }
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
    let status = tokio::process::Command::new("pkexec")
        .arg(exe)
        .args([
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
        ])
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
    // Windows has no pkexec/Authorization-Services: re-exec this binary elevated
    // via PowerShell's Start-Process -Verb RunAs (UAC) and wait for it.
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

/// Escape a string for embedding inside a single-quoted PowerShell literal
/// (the only special char inside `'...'` is a single quote, doubled).
#[cfg(target_os = "windows")]
fn ps_escape(s: &str) -> String {
    s.replace('\'', "''")
}

pub(crate) async fn decompress(
    app_handle: &AppHandle,
    src: &std::path::Path,
    dst: &std::path::Path,
    lower_name: &str,
    cancel: Arc<AtomicBool>,
) -> Result<u64, String> {
    emit_flash_progress(app_handle, FlashPhase::Decompressing, 0, 0, 0.0, 0);

    let src = src.to_path_buf();
    let dst = dst.to_path_buf();
    let app_handle = app_handle.clone();
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
                // total=0 → UI shows an indeterminate "decompressing N MB" bar.
                emit_flash_progress(
                    &app_handle,
                    FlashPhase::Decompressing,
                    bytes_written,
                    0,
                    0.0,
                    0,
                );
                last_emit = std::time::Instant::now();
            }
        }
        emit_flash_progress(
            &app_handle,
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

// ---------------------------------------------------------------------------
// Command: cancel_flash
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn cancel_flash(state: tauri::State<'_, FlasherState>) -> Result<(), String> {
    state.user_cancel.store(true, Ordering::SeqCst);
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flasher::isolated_dir;
    use crate::flasher::MIN_CARD_BYTES;

    /// The card has to hold the *decompressed* image. v0.3.2 is 586 MB
    /// compressed and 2.97 GB unpacked, so a check against the download size
    /// would clear a 1 GB card that cannot possibly hold it.
    #[test]
    fn card_check_uses_the_decompressed_size() {
        const UNCOMPRESSED: u64 = 2_969_567_232;
        const COMPRESSED: u64 = 586_187_284;
        const ONE_GB_CARD: u64 = 1_000_000_000;

        // The floor has to sit above a real image and above the cards that
        // comparing against the *download* size alone would wrongly clear.
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

    // A unique temp subdir per test *and* per process: a previous run whose
    // cleanup lost the race would otherwise leave a file behind and fail the
    // next run's `create_new`.

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
        // The secret-bearing blob must not be world/group readable.
        assert_eq!(mode & 0o777, 0o600, "provision file must be 0600");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sweep_removes_stale_provision_files() {
        let dir = isolated_dir("sweep-stale");
        let stale = dir.join(format!("{PROVISION_FILE_PREFIX}x.b64"));
        write_secret_file(&stale, "x").unwrap();
        // Zero threshold: every matching file counts as stale.
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
        // A just-written file is younger than the threshold: a concurrent
        // flash's in-flight blob must survive.
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
