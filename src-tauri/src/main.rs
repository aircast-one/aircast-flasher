// Prevent an extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(feature = "wdio", not(debug_assertions)))]
compile_error!("the wdio feature embeds a WebDriver server — never enable it in a release build");

mod flasher;
mod helper;
mod settings;
mod telemetry;
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

    let builder = tauri::Builder::default();

    #[cfg(feature = "wdio")]
    let builder = builder.plugin(tauri_plugin_wdio_webdriver::init());

    builder
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
            flasher::reveal_event_log,
            settings::read_settings,
            settings::write_settings,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
