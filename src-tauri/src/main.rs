// Prevent an extra console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(feature = "wdio", not(debug_assertions)))]
compile_error!("the wdio feature embeds a WebDriver server — never enable it in a release build");

mod connect;
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
        .setup(|app| {
            let handle = app.handle().clone();
            telemetry::record_panics(handle.clone());
            // A run that ended offline, or died before its last event shipped,
            // leaves the tail of the log unsent. This is where it catches up.
            telemetry::flush(&handle);
            Ok(())
        })
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
            connect::probe_device,
            connect::join_wifi,
            settings::read_settings,
            settings::write_settings,
            telemetry::track,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
