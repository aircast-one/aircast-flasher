use flasher_ops::tailnet::{self, LocalTailscale, TailscaleState};

#[tauri::command]
pub async fn probe_device(url: String) -> bool {
    tailnet::probe_device(url).await
}

#[tauri::command]
pub async fn probe_devices(urls: Vec<String>) -> Vec<String> {
    tailnet::probe_devices(urls).await
}

#[tauri::command]
pub async fn tailscale_login(url: String) -> Result<String, String> {
    tailnet::tailscale_login(url).await
}

#[tauri::command]
pub async fn tailscale_status(url: String) -> Result<TailscaleState, String> {
    tailnet::tailscale_status(url).await
}

#[tauri::command]
pub async fn set_device_control_server(url: String, control_server: String) -> Result<(), String> {
    tailnet::set_device_control_server(url, control_server).await
}

#[tauri::command]
pub async fn local_tailscale() -> LocalTailscale {
    tailnet::local_tailscale().await
}

#[tauri::command]
pub async fn join_wifi(ssid: String, password: String) -> Result<(), String> {
    tailnet::join_wifi(ssid, password).await
}

#[tauri::command]
pub async fn open_device_window(
    app: tauri::AppHandle,
    url: String,
    title: String,
) -> Result<(), String> {
    use tauri::Manager;

    let label = format!(
        "device-{}",
        title
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>()
    );

    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.unminimize();
        let _ = existing.show();
        return existing.set_focus().map_err(|e| e.to_string());
    }

    let target = url
        .parse()
        .map_err(|_| format!("Not a valid address: {url}"))?;

    tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::External(target))
        .title(title)
        .inner_size(1100.0, 760.0)
        .min_inner_size(700.0, 500.0)
        .build()
        .map(|_| ())
        .map_err(|e| e.to_string())
}
