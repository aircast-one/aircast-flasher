use serde::Serialize;

use super::http;
use super::ReleasesResponse;
use super::MIN_CARD_BYTES;

const CHANNEL_FALLBACK: &[&str] = &["stable", "staging", "development"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: &'static str,
    pub name: &'static str,
    pub maker: &'static str,
    pub detail: &'static str,
    pub builtin_wifi: bool,
    pub beta: bool,
    pub tested: bool,
    #[serde(skip)]
    catalog: &'static str,
}

const fn armbian(
    id: &'static str,
    name: &'static str,
    maker: &'static str,
    detail: &'static str,
    builtin_wifi: bool,
    catalog: &'static str,
) -> Board {
    Board {
        id,
        name,
        maker,
        detail,
        builtin_wifi,
        beta: true,
        tested: false,
        catalog,
    }
}

pub const BOARDS: &[Board] = &[
    Board {
        id: "rpi",
        name: "Raspberry Pi",
        maker: "Raspberry Pi",
        detail: "Pi 3, 4, 5 and Zero 2 W",
        builtin_wifi: true,
        beta: false,
        tested: true,
        catalog: "lite",
    },
    armbian(
        "orangepi5",
        "Orange Pi 5",
        "Orange Pi",
        "Rockchip RK3588S",
        false,
        "orangepi5",
    ),
    armbian(
        "orangepi5-plus",
        "Orange Pi 5 Plus",
        "Orange Pi",
        "Rockchip RK3588",
        false,
        "orangepi5-plus",
    ),
    armbian(
        "orangepi5pro",
        "Orange Pi 5 Pro",
        "Orange Pi",
        "Rockchip RK3588S",
        true,
        "orangepi5pro",
    ),
    armbian(
        "orangepi5-max",
        "Orange Pi 5 Max",
        "Orange Pi",
        "Rockchip RK3588",
        true,
        "orangepi5-max",
    ),
    armbian(
        "orangepi5-ultra",
        "Orange Pi 5 Ultra",
        "Orange Pi",
        "Rockchip RK3588",
        true,
        "orangepi5-ultra",
    ),
    armbian(
        "orangepi3b",
        "Orange Pi 3B",
        "Orange Pi",
        "Rockchip RK3566",
        true,
        "orangepi3b",
    ),
    armbian(
        "rock-5a",
        "Radxa ROCK 5A",
        "Radxa",
        "Rockchip RK3588S",
        false,
        "rock-5a",
    ),
    armbian(
        "rock-5b",
        "Radxa ROCK 5B",
        "Radxa",
        "Rockchip RK3588",
        false,
        "rock-5b",
    ),
    armbian(
        "rock-5b-plus",
        "Radxa ROCK 5B+",
        "Radxa",
        "Rockchip RK3588",
        true,
        "rock-5b-plus",
    ),
    armbian(
        "rock-5c",
        "Radxa ROCK 5C",
        "Radxa",
        "Rockchip RK3588S",
        true,
        "rock-5c",
    ),
    armbian(
        "rock-5t",
        "Radxa ROCK 5T",
        "Radxa",
        "Rockchip RK3588",
        false,
        "rock-5t",
    ),
    armbian(
        "rock-5-itx",
        "Radxa ROCK 5 ITX",
        "Radxa",
        "Rockchip RK3588",
        false,
        "rock-5-itx",
    ),
    armbian(
        "rock-3a",
        "Radxa ROCK 3A",
        "Radxa",
        "Rockchip RK3568",
        false,
        "rock-3a",
    ),
    armbian(
        "rock-3c",
        "Radxa ROCK 3C",
        "Radxa",
        "Rockchip RK3566",
        true,
        "rock-3c",
    ),
    Board {
        tested: true,
        ..armbian(
            "radxa-zero3w",
            "Radxa ZERO 3W",
            "Radxa",
            "Rockchip RK3566",
            true,
            "radxa-zero3",
        )
    },
    armbian(
        "radxa-zero3e",
        "Radxa ZERO 3E",
        "Radxa",
        "Rockchip RK3566",
        false,
        "radxa-zero3",
    ),
    armbian(
        "radxa-e52c",
        "Radxa E52C",
        "Radxa",
        "Rockchip RK3582",
        false,
        "radxa-e52c",
    ),
    armbian(
        "radxa-e25",
        "Radxa E25",
        "Radxa",
        "Rockchip RK3568",
        false,
        "radxa-e25",
    ),
    armbian(
        "bananapim7",
        "Banana Pi BPI-M7",
        "Banana Pi",
        "Rockchip RK3588",
        true,
        "bananapim7",
    ),
    armbian(
        "bananapir2pro",
        "Banana Pi BPI-R2 Pro",
        "Banana Pi",
        "Rockchip RK3568",
        false,
        "bananapir2pro",
    ),
    armbian(
        "bananapim5",
        "Banana Pi BPI-M5",
        "Banana Pi",
        "Amlogic S905X3",
        false,
        "bananapim5",
    ),
    armbian(
        "bananapim2pro",
        "Banana Pi BPI-M2 Pro",
        "Banana Pi",
        "Amlogic S905X3",
        true,
        "bananapim2pro",
    ),
    armbian(
        "bananapim2s",
        "Banana Pi BPI-M2S",
        "Banana Pi",
        "Amlogic A311D",
        false,
        "bananapim2s",
    ),
    armbian(
        "bananapicm4io",
        "Banana Pi BPI-CM4 + IO board",
        "Banana Pi",
        "Amlogic A311D",
        false,
        "bananapicm4io",
    ),
    armbian(
        "bananapim4zero",
        "Banana Pi BPI-M4 Zero",
        "Banana Pi",
        "Allwinner H618",
        true,
        "bananapim4zero",
    ),
    armbian(
        "bananapim4berry",
        "Banana Pi BPI-M4 Berry",
        "Banana Pi",
        "Allwinner H618",
        true,
        "bananapim4berry",
    ),
    armbian(
        "bananapim64",
        "Banana Pi BPI-M64",
        "Banana Pi",
        "Allwinner A64",
        true,
        "bananapim64",
    ),
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
    fn board_ids_are_unique_and_only_the_zero_3_variants_share_a_catalog() {
        let ids: std::collections::HashSet<_> = BOARDS.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), BOARDS.len());
        let shared: Vec<_> = BOARDS[1..]
            .iter()
            .filter(|b| b.catalog != b.id)
            .map(|b| (b.id, b.catalog))
            .collect();
        assert_eq!(
            shared,
            [
                ("radxa-zero3w", "radxa-zero3"),
                ("radxa-zero3e", "radxa-zero3")
            ]
        );
    }

    #[test]
    fn every_board_but_the_pi_is_beta_and_wifi_follows_the_hardware() {
        assert!(BOARDS[1..].iter().all(|b| b.beta) && !BOARDS[0].beta);
        let wifi = |id: &str| board(Some(id)).unwrap().builtin_wifi;
        assert!(wifi("rpi") && wifi("orangepi5pro") && wifi("radxa-zero3w"));
        assert!(!wifi("orangepi5") && !wifi("rock-5b") && !wifi("radxa-zero3e"));
    }

    #[test]
    fn a_board_reaches_the_app_in_camel_case_without_its_catalog() {
        let json = serde_json::to_value(BOARDS[1]).unwrap();
        assert_eq!(json["builtinWifi"], false);
        assert_eq!(json["maker"], "Orange Pi");
        assert_eq!(json["detail"], "Rockchip RK3588S");
        assert!(json.get("catalog").is_none());
    }

    #[test]
    fn only_the_zero_3w_among_the_beta_boards_has_run_on_real_hardware() {
        let tested: Vec<_> = BOARDS[1..]
            .iter()
            .filter(|b| b.tested)
            .map(|b| b.id)
            .collect();
        assert_eq!(tested, ["radxa-zero3w"]);
        assert!(BOARDS[0].tested);
        assert_eq!(serde_json::to_value(BOARDS[1]).unwrap()["tested"], false);
    }

    #[test]
    fn rejects_an_unknown_board_and_names_the_valid_ones() {
        let err = board(Some("jetson")).unwrap_err();
        assert!(
            err.contains("jetson") && err.contains("(expected rpi|") && err.contains("|rock-5b|"),
            "{err}"
        );
    }
}
