use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use flasher_ops::flash::{FlashEvent, FlashTrace};
use flasher_ops::{
    BlockDevice, DownloadProgress, DownloadResult, ProvisionConfig, ReleasesResponse,
    TailscaleConfig, WifiConfig,
};
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_dialog::DialogExt;

use crate::telemetry;

pub struct FlasherState {
    pub cancel: Arc<AtomicBool>,
    pub user_cancel: Arc<AtomicBool>,
}

impl FlasherState {
    pub fn new() -> Self {
        Self {
            cancel: Arc::new(AtomicBool::new(false)),
            user_cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    fn start(&self) -> Arc<AtomicBool> {
        self.user_cancel.store(false, Ordering::SeqCst);
        self.cancel.store(false, Ordering::SeqCst);
        self.cancel.clone()
    }
}

fn outcome<T>(result: &Result<T, String>, cancelled: &AtomicBool) -> &'static str {
    match result {
        Ok(_) => "success",
        Err(_) if cancelled.load(Ordering::SeqCst) => "cancelled",
        Err(_) => "failed",
    }
}

#[tauri::command]
pub async fn list_releases(
    channel: Option<String>,
    board: Option<String>,
) -> Result<ReleasesResponse, String> {
    flasher_ops::releases::list_releases(channel, board).await
}

#[tauri::command]
pub fn list_boards() -> Vec<flasher_ops::releases::Board> {
    flasher_ops::releases::BOARDS.to_vec()
}

#[tauri::command]
pub async fn list_running_boards() -> Vec<String> {
    flasher_ops::usbboard::running_boards().await
}

#[tauri::command]
pub async fn list_block_devices() -> Result<Vec<BlockDevice>, String> {
    flasher_ops::devices::list_block_devices().await
}

#[tauri::command]
pub async fn download_image(
    app_handle: AppHandle,
    state: State<'_, FlasherState>,
    job_id: String,
    download_url: String,
    checksum_url: String,
) -> Result<DownloadResult, String> {
    let started = std::time::Instant::now();
    let cancel = state.start();
    let mut stats = flasher_ops::download::DownloadStats::default();
    let mut emit = |p: &DownloadProgress| {
        let _ = app_handle.emit("flasher:download-progress", p);
    };
    let result = match flasher_ops::paths::image_cache_dir() {
        Ok(cache_dir) => {
            flasher_ops::download::download_image(
                &cache_dir,
                &download_url,
                &checksum_url,
                &cancel,
                &mut emit,
                &mut stats,
            )
            .await
        }
        Err(e) => Err(e),
    };
    let duration_ms = started.elapsed().as_millis() as u64;

    let bytes = result
        .as_ref()
        .ok()
        .and_then(|r| std::fs::metadata(&r.image_path).ok().map(|m| m.len()))
        .unwrap_or(0);
    let cached = result.as_ref().map(|r| r.cached).unwrap_or(false);

    telemetry::record(
        &app_handle,
        &telemetry::DownloadEvent {
            envelope: telemetry::Envelope::new(
                "download",
                job_id,
                &app_handle,
                outcome(&result, &state.user_cancel),
                duration_ms,
                result.as_ref().err().cloned(),
            ),
            bytes,
            cached,
            mbps: (!cached)
                .then(|| telemetry::mbps(bytes, stats.transfer_ms))
                .flatten(),
            retries: stats.retries,
            restarts: stats.restarts,
            resumed_bytes: stats.resumed_bytes,
        },
    );

    result
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn flash_image(
    app_handle: AppHandle,
    state: State<'_, FlasherState>,
    job_id: String,
    image_path: String,
    target_disk: String,
    wifi: Option<WifiConfig>,
    hostname: Option<String>,
    tailscale: Option<TailscaleConfig>,
    access: Option<flasher_ops::AccessConfig>,
    encryption: Option<flasher_ops::EncryptionConfig>,
    fight_mode: bool,
    init_format: flasher_ops::InitFormat,
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
    let phases = Arc::new(Mutex::new(telemetry::PhaseLog::default()));
    let started = std::time::Instant::now();
    let mut trace = FlashTrace::default();

    let events = {
        let app = app_handle.clone();
        let phases = phases.clone();
        Arc::new(move |event: FlashEvent| match event {
            FlashEvent::Progress(progress) => {
                let _ = app.emit("flasher:flash-progress", progress);
            }
            FlashEvent::Stage { stage, bytes } => {
                if let Ok(mut log) = phases.lock() {
                    log.mark(stage, bytes);
                }
            }
            FlashEvent::Diag(line) => {
                if let Ok(mut log) = phases.lock() {
                    log.diag.push(line);
                }
            }
        })
    };

    let result = flasher_ops::flash::flash_image(
        &config,
        &image_path,
        &target_disk,
        state.start(),
        events,
        &mut trace,
    )
    .await;

    let end = std::time::Instant::now();
    let duration_ms = started.elapsed().as_millis() as u64;
    let log = phases.lock().expect("phase log poisoned");
    let write_ms = log.duration_ms("write", end);
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
                outcome(&result, &state.user_cancel),
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

#[tauri::command]
pub async fn cancel_flash(state: State<'_, FlasherState>) -> Result<(), String> {
    state.user_cancel.store(true, Ordering::SeqCst);
    state.cancel.store(true, Ordering::SeqCst);
    Ok(())
}

#[tauri::command]
pub async fn fetch_tang_adv(url: String) -> Result<String, String> {
    flasher_ops::tang::fetch_tang_adv(url).await
}

#[tauri::command]
pub async fn save_recovery_file(
    app: AppHandle,
    default_name: String,
    contents: String,
) -> Result<bool, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(&default_name)
        .add_filter("Text", &["txt"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let Some(path) = rx.await.map_err(|e| e.to_string())? else {
        return Ok(false);
    };
    let path = path.into_path().map_err(|e| e.to_string())?;
    std::fs::write(&path, contents)
        .map_err(|e| format!("could not write {}: {e}", path.display()))?;
    Ok(true)
}

#[tauri::command]
pub async fn list_wifi_networks() -> Result<flasher_ops::wifi::WifiNetworks, String> {
    flasher_ops::wifi::list_wifi_networks().await
}

#[tauri::command]
pub fn detect_ssh_keys() -> Vec<flasher_ops::sshkeys::SshPublicKey> {
    flasher_ops::sshkeys::detect_ssh_keys()
}

#[tauri::command]
pub fn read_public_key(path: String) -> Result<String, String> {
    flasher_ops::sshkeys::read_public_key(path)
}

#[tauri::command]
pub fn identify_public_key(key: String) -> Option<flasher_ops::sshkeys::SshKeyIdentity> {
    flasher_ops::sshkeys::identify_public_key(key)
}
