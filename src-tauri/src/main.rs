// Prevent an extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod flasher;
mod helper;
#[cfg(target_os = "macos")]
mod macos_auth;

use flasher::FlasherState;

#[tokio::main]
async fn main() {
    // The elevated flash helper is this same binary re-executed (Touch ID on
    // macOS / UAC on Windows / pkexec on Linux). It opens the raw device as
    // root, runs the engine, and exits before any GUI is constructed.
    if std::env::args().any(|a| a == "--flash-helper") {
        helper::run_flash_helper();
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .manage(FlasherState::new())
        .invoke_handler(tauri::generate_handler![
            flasher::list_releases,
            flasher::list_block_devices,
            flasher::download_image,
            flasher::flash_image,
            flasher::list_wifi_networks,
            flasher::cancel_flash,
            flasher::detect_ssh_keys,
            flasher::read_public_key,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
