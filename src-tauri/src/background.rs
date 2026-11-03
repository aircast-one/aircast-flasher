use std::sync::atomic::{AtomicBool, Ordering};

use tauri::menu::{Menu, MenuItem};
use tauri::plugin::TauriPlugin;
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Manager, Runtime, Window, WindowEvent};
use tauri_plugin_autostart::ManagerExt;

const STARTED_AT_LOGIN: &str = "--background";
const TRAY_ID: &str = "background";
const MAIN_WINDOW: &str = "main";

#[derive(Default)]
pub struct Tray(AtomicBool);

#[derive(Debug, PartialEq, Eq)]
enum OnClose {
    Quit,
    Hide,
    Minimize,
}

fn on_close(background: bool, tray: bool) -> OnClose {
    match (background, tray) {
        (false, _) => OnClose::Quit,
        (true, true) => OnClose::Hide,
        (true, false) => OnClose::Minimize,
    }
}

fn started_at_login(mut args: impl Iterator<Item = String>) -> bool {
    args.any(|arg| arg == STARTED_AT_LOGIN)
}

pub fn autostart<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_autostart::Builder::new()
        .arg(STARTED_AT_LOGIN)
        .build()
}

pub fn setup(app: &AppHandle) {
    if !crate::settings::background_backups(app) {
        return;
    }
    show_tray(app);
    if started_at_login(std::env::args()) {
        hide_main(app);
    }
}

pub fn on_window_event(window: &Window, event: &WindowEvent) {
    let WindowEvent::CloseRequested { api, .. } = event else {
        return;
    };
    if window.label() != MAIN_WINDOW {
        return;
    }
    let app = window.app_handle();
    match on_close(
        crate::settings::background_backups(app),
        app.state::<Tray>().0.load(Ordering::SeqCst),
    ) {
        OnClose::Quit => {}
        OnClose::Hide => {
            api.prevent_close();
            hide_main(app);
        }
        OnClose::Minimize => {
            api.prevent_close();
            let _ = window.minimize();
        }
    }
}

pub fn show_main(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

fn hide_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN_WINDOW) {
        let _ = window.hide();
    }
    #[cfg(target_os = "macos")]
    let _ = app.set_activation_policy(tauri::ActivationPolicy::Accessory);
}

#[cfg(target_os = "linux")]
fn tray_supported() -> bool {
    ["libayatana-appindicator3.so.1", "libappindicator3.so.1"]
        .iter()
        .any(|lib| unsafe { libloading::Library::new(lib) }.is_ok())
}

#[cfg(not(target_os = "linux"))]
fn tray_supported() -> bool {
    true
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &quit])?;
    let tray = TrayIconBuilder::with_id(TRAY_ID)
        .menu(&menu)
        .tooltip("Aircast Flasher — backing up your drones")
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => show_main(app),
            "quit" => app.exit(0),
            _ => {}
        });
    match app.default_window_icon() {
        Some(icon) => tray.icon(icon.clone()),
        None => tray,
    }
    .build(app)
    .map(|_| ())
}

fn show_tray(app: &AppHandle) -> bool {
    let shown = app.tray_by_id(TRAY_ID).is_some() || (tray_supported() && build_tray(app).is_ok());
    app.state::<Tray>().0.store(shown, Ordering::SeqCst);
    shown
}

fn hide_tray(app: &AppHandle) {
    let _ = app.remove_tray_by_id(TRAY_ID);
    app.state::<Tray>().0.store(false, Ordering::SeqCst);
}

#[tauri::command]
pub fn set_background_backups(app: AppHandle, on: bool) -> Result<bool, String> {
    let autostart = app.autolaunch();
    let registered = autostart.is_enabled().unwrap_or(false);
    match (on, registered) {
        (true, false) => autostart.enable(),
        (false, true) => autostart.disable(),
        _ => Ok(()),
    }
    .map_err(|e| format!("Couldn't change whether the Flasher starts with your computer: {e}"))?;
    Ok(match on {
        true => show_tray(&app),
        false => {
            hide_tray(&app);
            false
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_window_quits_unless_backing_up_in_the_background() {
        assert_eq!(on_close(false, true), OnClose::Quit);
        assert_eq!(on_close(false, false), OnClose::Quit);
        assert_eq!(on_close(true, true), OnClose::Hide);
        assert_eq!(on_close(true, false), OnClose::Minimize);
    }

    #[test]
    fn only_a_login_start_opens_hidden() {
        let args = |list: &[&str]| list.iter().map(|a| a.to_string()).collect::<Vec<_>>();
        assert!(started_at_login(
            args(&["aircast-flasher", "--background"]).into_iter()
        ));
        assert!(!started_at_login(args(&["aircast-flasher"]).into_iter()));
        assert!(!started_at_login(
            args(&["aircast-flasher", "--flash-helper"]).into_iter()
        ));
    }
}
