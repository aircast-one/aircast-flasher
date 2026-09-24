use super::http;
use super::ReleasesResponse;
use super::MIN_CARD_BYTES;

// ---------------------------------------------------------------------------
// Command: list_releases
// ---------------------------------------------------------------------------

/// Channels in preference order, used when no explicit channel is requested.
/// `stable` is preferred but may have no published release yet, so we fall
/// through to `staging` then `development`.
const CHANNEL_FALLBACK: &[&str] = &["stable", "staging", "development"];

fn channel_host(channel: &str) -> Result<&'static str, String> {
    match channel {
        "stable" => Ok("downloads.aircast.one"),
        "development" => Ok("downloads-dev.aircast.one"),
        "staging" => Ok("downloads.stage.aircast.one"),
        other => Err(format!(
            "Unknown channel: {other} (expected stable|development|staging)"
        )),
    }
}

async fn fetch_releases(channel: &str) -> Result<ReleasesResponse, String> {
    let host = channel_host(channel)?;
    let url = format!("https://{host}/lite/releases.json");
    let response = http()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch releases: {e}"))?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }
    response
        .json::<ReleasesResponse>()
        .await
        .map(|r| ReleasesResponse {
            min_card_bytes: MIN_CARD_BYTES,
            ..r
        })
        .map_err(|e| format!("Failed to parse releases: {e}"))
}

#[tauri::command]
pub async fn list_releases(channel: Option<String>) -> Result<ReleasesResponse, String> {
    // Explicit channel: use only that one.
    if let Some(ch) = channel {
        return fetch_releases(&ch).await;
    }

    // No channel: prefer stable, fall back to staging/development so the app
    // works before a stable image is cut, and auto-upgrades once it is.
    let mut last_err = "no channels tried".to_string();
    for ch in CHANNEL_FALLBACK {
        match fetch_releases(ch).await {
            Ok(r) if !r.releases.is_empty() => return Ok(r),
            Ok(_) => last_err = format!("{ch}: empty"),
            Err(e) => last_err = format!("{ch}: {e}"),
        }
    }
    Err(format!("No releases available on any channel ({last_err})"))
}
