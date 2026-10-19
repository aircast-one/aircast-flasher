#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(feature = "wdio", not(debug_assertions)))]
compile_error!("the wdio feature embeds a WebDriver server — never enable it in a release build");

mod cloud;
mod flasher;
mod settings;
mod tailnet;
mod telemetry;

use flasher::FlasherState;

#[tokio::main]
async fn main() {
    if std::env::args().any(|a| a == "--flash-helper") {
        flasher_ops::helper::run_flash_helper();
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
        .manage(cloud::CloudState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            telemetry::record_panics(handle.clone());
            telemetry::flush(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            flasher::list_releases,
            flasher::list_block_devices,
            flasher::download_image,
            flasher::flash_image,
            flasher::fetch_tang_adv,
            flasher::save_recovery_file,
            cloud::cloud_sign_in_start,
            cloud::cloud_sign_in_poll,
            cloud::cloud_account,
            cloud::cloud_sign_out,
            cloud::cloud_create_keyset,
            cloud::cloud_list_cards,
            cloud::cloud_mark_lost,
            flasher::list_wifi_networks,
            flasher::cancel_flash,
            flasher::detect_ssh_keys,
            flasher::read_public_key,
            flasher::identify_public_key,
            telemetry::reveal_event_log,
            tailnet::probe_device,
            tailnet::probe_devices,
            tailnet::open_device_window,
            tailnet::set_device_control_server,
            tailnet::tailscale_login,
            tailnet::tailscale_status,
            tailnet::local_tailscale,
            tailnet::join_wifi,
            settings::read_settings,
            settings::update_settings,
            telemetry::track,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
