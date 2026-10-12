use super::http;
use super::ReleasesResponse;
use super::MIN_CARD_BYTES;

const RELEASES_URL: &str = "https://downloads.aircast.one/lite/releases.json";

#[tauri::command]
pub async fn list_releases() -> Result<ReleasesResponse, String> {
    let response = http()
        .get(RELEASES_URL)
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
