#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(all(feature = "wdio", not(debug_assertions)))]
compile_error!("the wdio feature embeds a WebDriver server — never enable it in a release build");

mod cloud;
mod connect;
mod flasher;
mod helper;
#[cfg(target_os = "macos")]
mod macos_auth;
mod settings;
mod telemetry;

use flasher::FlasherState;

#[tokio::main]
async fn main() {
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
        .manage(cloud::CloudState::default())
        .setup(|app| {
            let handle = app.handle().clone();
            telemetry::record_panics(handle.clone());
            telemetry::flush(&handle);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            flasher::releases::list_releases,
            flasher::devices::list_block_devices,
            flasher::download::download_image,
            flasher::flash::flash_image,
            flasher::encrypt::fetch_tang_adv,
            flasher::encrypt::save_recovery_file,
            cloud::cloud_sign_in_start,
            cloud::cloud_sign_in_poll,
            cloud::cloud_account,
            cloud::cloud_sign_out,
            cloud::cloud_create_keyset,
            cloud::cloud_list_cards,
            cloud::cloud_mark_lost,
            flasher::wifi::list_wifi_networks,
            flasher::flash::cancel_flash,
            flasher::sshkeys::detect_ssh_keys,
            flasher::sshkeys::read_public_key,
            flasher::sshkeys::identify_public_key,
            telemetry::reveal_event_log,
            connect::probe_device,
            connect::probe_devices,
            connect::open_device_window,
            connect::set_device_control_server,
            connect::tailscale_login,
            connect::tailscale_status,
            connect::local_tailscale,
            connect::join_wifi,
            settings::read_settings,
            settings::write_settings,
            telemetry::track,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
