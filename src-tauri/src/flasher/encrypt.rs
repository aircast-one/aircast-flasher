//! Disk-encryption helper: fetch a Tang server's advertisement so the flasher
//! can enrol a card offline. The advertisement is public (a signed JWS of the
//! server's keys), not a secret — it only has to reach the device at every
//! later boot to unlock the card.

use std::time::Duration;

use tauri_plugin_dialog::DialogExt;

/// Fetch `<url>/adv` from a Tang server and return it verbatim. The device
/// binds to this offline during first boot, so it must be the real
/// advertisement, not an error page: a JWS General JSON Serialization carrying
/// `payload` and `signatures`.
#[tauri::command]
pub async fn fetch_tang_adv(url: String) -> Result<String, String> {
    let base = url.trim().trim_end_matches('/');
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err("Tang URL must start with http:// or https://".into());
    }
    let adv_url = format!("{base}/adv");

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let resp = client
        .get(&adv_url)
        .send()
        .await
        .map_err(|e| format!("could not reach Tang at {adv_url}: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("Tang returned {} for {adv_url}", resp.status()));
    }
    let body = resp
        .text()
        .await
        .map_err(|e| format!("reading Tang advertisement: {e}"))?;
    validate_adv(&body)?;
    Ok(body)
}

/// Show a save dialog and write `contents` to the chosen path. The dialog and
/// the write both live in the backend, so the webview never supplies a path —
/// there is no arbitrary-path write primitive exposed to it. Returns `true` if
/// the operator saved, `false` if they cancelled. Used to export the recovery
/// key so it isn't trapped in a copy-once wizard field.
#[tauri::command]
pub async fn save_recovery_file(
    app: tauri::AppHandle,
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

/// A Tang advertisement is a JWS in General JSON Serialization: an object with a
/// `payload` and a `signatures` array. Reject anything else so a captive-portal
/// page or a wrong URL can't be written onto the card as the binding material.
fn validate_adv(body: &str) -> Result<(), String> {
    let v: serde_json::Value =
        serde_json::from_str(body).map_err(|_| "Tang advertisement is not JSON".to_string())?;
    let has_payload = v.get("payload").and_then(|p| p.as_str()).is_some();
    let has_sigs = v
        .get("signatures")
        .and_then(|s| s.as_array())
        .is_some_and(|a| !a.is_empty());
    if has_payload && has_sigs {
        Ok(())
    } else {
        Err("response is not a Tang advertisement (no payload/signatures)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_jws_shaped_advertisement() {
        let adv = r#"{"payload":"eyJrZXlzIjpbXX0","protected":"e30","signatures":[{"signature":"abc"}]}"#;
        assert!(validate_adv(adv).is_ok());
    }

    #[test]
    fn rejects_non_advertisement_responses() {
        assert!(validate_adv("<html>404</html>").is_err());
        assert!(validate_adv(r#"{"payload":"x"}"#).is_err());
        assert!(validate_adv(r#"{"signatures":[{"signature":"a"}]}"#).is_err());
        assert!(validate_adv(r#"{"payload":"x","signatures":[]}"#).is_err());
    }
}
