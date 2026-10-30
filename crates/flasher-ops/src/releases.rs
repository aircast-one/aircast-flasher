use serde::Serialize;

use super::http;
use super::ReleasesResponse;
use super::MIN_CARD_BYTES;

const CHANNEL_FALLBACK: &[&str] = &["stable", "staging", "development"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Board {
    pub id: &'static str,
    pub name: &'static str,
    #[serde(skip)]
    catalog: &'static str,
}

pub const BOARDS: &[Board] = &[
    Board {
        id: "rpi",
        name: "Raspberry Pi",
        catalog: "lite",
    },
    Board {
        id: "orangepi5",
        name: "Orange Pi 5",
        catalog: "orangepi5",
    },
    Board {
        id: "rock-5b",
        name: "Radxa ROCK 5B",
        catalog: "rock-5b",
    },
];

pub fn board(id: Option<&str>) -> Result<&'static Board, String> {
    let id = id.unwrap_or(BOARDS[0].id);
    BOARDS.iter().find(|b| b.id == id).ok_or_else(|| {
        let ids: Vec<_> = BOARDS.iter().map(|b| b.id).collect();
        format!("Unknown board: {id} (expected {})", ids.join("|"))
    })
}

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

async fn fetch_releases(channel: &str, board: &Board) -> Result<ReleasesResponse, String> {
    let host = channel_host(channel)?;
    let url = format!("https://{host}/{}/releases.json", board.catalog);
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
pub async fn list_releases(
    channel: Option<String>,
    board_id: Option<String>,
) -> Result<ReleasesResponse, String> {
    let board = board(board_id.as_deref())?;
    if let Some(ch) = channel {
        return fetch_releases(&ch, board).await;
    }

    let mut last_err = "no channels tried".to_string();
    for ch in CHANNEL_FALLBACK {
        match fetch_releases(ch, board).await {
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

    #[test]
    fn the_pi_is_the_default_board_and_keeps_the_lite_catalog() {
        let pi = board(None).unwrap();
        assert_eq!((pi.id, pi.catalog), ("rpi", "lite"));
        assert_eq!(board(Some("rpi")).unwrap(), pi);
    }

    #[test]
    fn every_other_board_is_served_from_its_own_catalog() {
        assert!(BOARDS[1..].iter().all(|b| b.catalog == b.id));
        let ids: std::collections::HashSet<_> = BOARDS.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), BOARDS.len());
    }

    #[test]
    fn rejects_an_unknown_board_and_names_the_valid_ones() {
        let err = board(Some("jetson")).unwrap_err();
        assert!(
            err.contains("jetson") && err.contains("rpi|orangepi5|rock-5b"),
            "{err}"
        );
    }
}
