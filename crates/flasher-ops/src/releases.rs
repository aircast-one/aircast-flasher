use super::http;
use super::ReleasesResponse;
use super::MIN_CARD_BYTES;

const CHANNEL_FALLBACK: &[&str] = &["stable", "staging", "development"];

fn channel_host(channel: &str) -> Result<&'static str, String> {
    match channel {
        "stable" => Ok("downloads.aircast.one"),
        "development" => Ok("downloads-dev.aircast.one"),
        "staging" => Ok("downloads-stage.aircast.one"),
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
pub async fn list_releases(channel: Option<String>) -> Result<ReleasesResponse, String> {
    if let Some(ch) = channel {
        return fetch_releases(&ch).await;
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_channel_is_served_from_a_one_label_aircast_host() {
        let hosts: Vec<_> = CHANNEL_FALLBACK
            .iter()
            .map(|ch| channel_host(ch).unwrap())
            .collect();
        assert_eq!(
            hosts,
            [
                "downloads.aircast.one",
                "downloads-stage.aircast.one",
                "downloads-dev.aircast.one"
            ]
        );
        assert!(hosts.iter().all(|h| h
            .strip_suffix(".aircast.one")
            .is_some_and(|l| !l.contains('.'))));
    }

    #[test]
    fn rejects_an_unknown_channel() {
        assert!(channel_host("nightly").is_err());
    }
}
