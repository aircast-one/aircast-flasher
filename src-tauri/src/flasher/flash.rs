use super::check_available_space;
use super::devices::list_block_devices;
use super::emit_flash_progress;
use super::fits_on_card;
#[cfg(target_os = "windows")]
use super::hidden_command;
use super::private_temp_dir;
use super::randbits;
use super::validate_disk_path;
use super::validate_fight_access;
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
    encryption: Option<flasher_core::EncryptionConfig>,
    fight_mode: bool,
    init_format: flasher_core::InitFormat,
) -> Result<(), String> {
    let config = ProvisionConfig {
        hostname,
        wifi,
        tailscale,
        access,
        encryption,
        fight_mode,
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
    validate_fight_access(config)?;
    let src_path = validate_image_file(std::path::Path::new(&image_path))?;

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
    app_handle: AppHandle,
    target_disk: String,
    image: std::path::PathBuf,
    ready_file: std::path::PathBuf,
    provision_b64: String,
    phases: Arc<std::sync::Mutex<telemetry::PhaseLog>>,
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
