use serde::{Deserialize, Serialize};

use super::http;
use super::ImageInfo;
use super::Release;
use super::ReleasesResponse;
use super::MIN_CARD_BYTES;

const CHANNEL_FALLBACK: &[&str] = &["stable", "staging", "development"];
const PI_REPO: &str = "aircast-one/aircast-pi-gen";
const BOARD_REPO: &str = "aircast-one/aircast-armbian";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Board {
    pub id: &'static str,
    pub name: &'static str,
    pub maker: &'static str,
    pub detail: &'static str,
    pub builtin_wifi: bool,
    pub beta: bool,
    #[serde(skip)]
    repo: &'static str,
    #[serde(skip)]
    image: &'static str,
}

const fn rockchip(
    id: &'static str,
    name: &'static str,
    maker: &'static str,
    detail: &'static str,
    builtin_wifi: bool,
    image: &'static str,
) -> Board {
    Board {
        id,
        name,
        maker,
        detail,
        builtin_wifi,
        beta: true,
        repo: BOARD_REPO,
        image,
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
        repo: PI_REPO,
        image: "aircast-lite-arm64",
    },
    rockchip(
        "orangepi5",
        "Orange Pi 5",
        "Orange Pi",
        "Rockchip RK3588S",
        false,
        "aircast-orangepi5",
    ),
    rockchip(
        "orangepi5-plus",
        "Orange Pi 5 Plus",
        "Orange Pi",
        "Rockchip RK3588",
        false,
        "aircast-orangepi5-plus",
    ),
    rockchip(
        "orangepi5pro",
        "Orange Pi 5 Pro",
        "Orange Pi",
        "Rockchip RK3588S",
        true,
        "aircast-orangepi5pro",
    ),
    rockchip(
        "orangepi5-max",
        "Orange Pi 5 Max",
        "Orange Pi",
        "Rockchip RK3588",
        true,
        "aircast-orangepi5-max",
    ),
    rockchip(
        "orangepi5-ultra",
        "Orange Pi 5 Ultra",
        "Orange Pi",
        "Rockchip RK3588",
        true,
        "aircast-orangepi5-ultra",
    ),
    rockchip(
        "orangepi3b",
        "Orange Pi 3B",
        "Orange Pi",
        "Rockchip RK3566",
        true,
        "aircast-orangepi3b",
    ),
    rockchip(
        "rock-5a",
        "Radxa ROCK 5A",
        "Radxa",
        "Rockchip RK3588S",
        false,
        "aircast-rock-5a",
    ),
    rockchip(
        "rock-5b",
        "Radxa ROCK 5B",
        "Radxa",
        "Rockchip RK3588",
        false,
        "aircast-rock-5b",
    ),
    rockchip(
        "rock-5b-plus",
        "Radxa ROCK 5B+",
        "Radxa",
        "Rockchip RK3588",
        true,
        "aircast-rock-5b-plus",
    ),
    rockchip(
        "rock-5c",
        "Radxa ROCK 5C",
        "Radxa",
        "Rockchip RK3588S",
        true,
        "aircast-rock-5c",
    ),
    rockchip(
        "rock-5t",
        "Radxa ROCK 5T",
        "Radxa",
        "Rockchip RK3588",
        false,
        "aircast-rock-5t",
    ),
    rockchip(
        "rock-5-itx",
        "Radxa ROCK 5 ITX",
        "Radxa",
        "Rockchip RK3588",
        false,
        "aircast-rock-5-itx",
    ),
    rockchip(
        "rock-3a",
        "Radxa ROCK 3A",
        "Radxa",
        "Rockchip RK3568",
        false,
        "aircast-rock-3a",
    ),
    rockchip(
        "rock-3c",
        "Radxa ROCK 3C",
        "Radxa",
        "Rockchip RK3566",
        true,
        "aircast-rock-3c",
    ),
    rockchip(
        "radxa-zero3w",
        "Radxa ZERO 3W",
        "Radxa",
        "Rockchip RK3566",
        true,
        "aircast-radxa-zero3",
    ),
    rockchip(
        "radxa-zero3e",
        "Radxa ZERO 3E",
        "Radxa",
        "Rockchip RK3566",
        false,
        "aircast-radxa-zero3",
    ),
    rockchip(
        "radxa-e52c",
        "Radxa E52C",
        "Radxa",
        "Rockchip RK3582",
        false,
        "aircast-radxa-e52c",
    ),
    rockchip(
        "radxa-e25",
        "Radxa E25",
        "Radxa",
        "Rockchip RK3568",
        false,
        "aircast-radxa-e25",
    ),
];

pub fn board(id: Option<&str>) -> Result<&'static Board, String> {
    let id = id.unwrap_or(BOARDS[0].id);
    BOARDS.iter().find(|b| b.id == id).ok_or_else(|| {
        let ids: Vec<_> = BOARDS.iter().map(|b| b.id).collect();
        format!("Unknown board: {id} (expected {})", ids.join("|"))
    })
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    prerelease: bool,
    draft: bool,
    created_at: String,
    published_at: Option<String>,
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    size: u64,
    browser_download_url: String,
}

fn known_channel(channel: &str) -> Result<(), String> {
    CHANNEL_FALLBACK
        .contains(&channel)
        .then_some(())
        .ok_or_else(|| format!("Unknown channel: {channel} (expected stable|development|staging)"))
}

fn channel_of(version: &str) -> &'static str {
    if version.contains("-dev.") {
        "development"
    } else if version.contains("-stage.") {
        "staging"
    } else {
        "stable"
    }
}

fn release_for(board: &Board, release: &GithubRelease) -> Option<Release> {
    let filename = format!("{}-{}.img.xz", board.image, release.tag_name);
    let checksum = format!("{filename}.sha256");
    let asset = |name: &str| release.assets.iter().find(|a| a.name == name);
    let (image, sum) = (asset(&filename)?, asset(&checksum)?);
    (!release.draft).then(|| Release {
        version: release.tag_name.clone(),
        prerelease: release.prerelease,
        created_at: release
            .published_at
            .clone()
            .unwrap_or_else(|| release.created_at.clone()),
        image: ImageInfo {
            filename: filename.clone(),
            extension: "img.xz".to_string(),
            size: image.size,
            download_url: image.browser_download_url.clone(),
            checksum_url: sum.browser_download_url.clone(),
        },
    })
}

fn pick_channel(releases: Vec<Release>, channel: Option<&str>) -> Vec<Release> {
    let on = |ch: &str| -> Vec<Release> {
        releases
            .iter()
            .filter(|r| channel_of(&r.version) == ch)
            .cloned()
            .collect()
    };
    match channel {
        Some(ch) => on(ch),
        None => CHANNEL_FALLBACK
            .iter()
            .map(|ch| on(ch))
            .find(|found| !found.is_empty())
            .unwrap_or_default(),
    }
}

async fn github_releases(repo: &str) -> Result<Vec<GithubRelease>, String> {
    let url = format!("https://api.github.com/repos/{repo}/releases?per_page=100");
    let response = http()
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|e| format!("Failed to fetch releases: {e}"))?;
    match response.status().as_u16() {
        200 => response
            .json::<Vec<GithubRelease>>()
            .await
            .map_err(|e| format!("Failed to parse releases: {e}")),
        403 | 429 => Err(
            "GitHub is limiting how often this computer can check for releases. Try again in a few minutes"
                .to_string(),
        ),
        status => Err(format!("Failed to fetch releases: HTTP {status}")),
    }
}

pub async fn list_releases(
    channel: Option<String>,
    board_id: Option<String>,
) -> Result<ReleasesResponse, String> {
    let board = board(board_id.as_deref())?;
    channel.as_deref().map(known_channel).transpose()?;
    let found: Vec<Release> = github_releases(board.repo)
        .await?
        .iter()
        .filter_map(|r| release_for(board, r))
        .collect();
    let releases = pick_channel(found, channel.as_deref());
    if releases.is_empty() {
        return Err(format!("No {} releases are published yet", board.name));
    }
    Ok(ReleasesResponse {
        releases,
        min_card_bytes: MIN_CARD_BYTES,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn github(tag: &str, draft: bool, assets: &[&str]) -> GithubRelease {
        GithubRelease {
            tag_name: tag.to_string(),
            prerelease: tag.contains('-'),
            draft,
            created_at: "2026-10-30T10:00:00Z".to_string(),
            published_at: Some("2026-10-31T10:00:00Z".to_string()),
            assets: assets
                .iter()
                .map(|name| GithubAsset {
                    name: name.to_string(),
                    size: 945,
                    browser_download_url: format!(
                        "https://github.com/x/releases/download/{tag}/{name}"
                    ),
                })
                .collect(),
        }
    }

    #[test]
    fn a_board_is_offered_its_own_image_with_the_checksum_beside_it() {
        let release = github(
            "v0.4.0-beta.4",
            false,
            &[
                "aircast-radxa-zero3-v0.4.0-beta.4.img.xz",
                "aircast-radxa-zero3-v0.4.0-beta.4.img.xz.sha256",
                "aircast-orangepi5-v0.4.0-beta.4.img.xz",
            ],
        );
        let zero = release_for(board(Some("radxa-zero3e")).unwrap(), &release).unwrap();
        assert_eq!(zero.version, "v0.4.0-beta.4");
        assert!(zero.prerelease);
        assert_eq!(zero.created_at, "2026-10-31T10:00:00Z");
        assert_eq!(
            zero.image.filename,
            "aircast-radxa-zero3-v0.4.0-beta.4.img.xz"
        );
        assert_eq!(zero.image.size, 945);
        assert!(zero.image.checksum_url.ends_with(".img.xz.sha256"));
        assert!(
            release_for(board(Some("orangepi5")).unwrap(), &release).is_none(),
            "an image without its checksum is not offered"
        );
        assert!(release_for(board(Some("rock-5b")).unwrap(), &release).is_none());
    }

    #[test]
    fn a_draft_release_is_never_offered() {
        let release = github(
            "v0.3.5",
            true,
            &[
                "aircast-lite-arm64-v0.3.5.img.xz",
                "aircast-lite-arm64-v0.3.5.img.xz.sha256",
            ],
        );
        assert!(release_for(board(None).unwrap(), &release).is_none());
    }

    #[test]
    fn the_tag_names_the_channel() {
        let channels: Vec<_> = [
            "v0.3.5",
            "v0.4.0-beta.4",
            "v0.4.6-rc.1",
            "v0.4.0-stage.1",
            "v0.2.1-dev.7",
        ]
        .iter()
        .map(|v| channel_of(v))
        .collect();
        assert_eq!(
            channels,
            ["stable", "stable", "stable", "staging", "development"]
        );
        assert!(known_channel("staging").is_ok() && known_channel("nightly").is_err());
    }

    #[test]
    fn without_a_channel_the_first_one_with_releases_wins() {
        let pi = board(None).unwrap();
        let of = |tags: &[&str]| -> Vec<Release> {
            tags.iter()
                .filter_map(|t| {
                    let name = format!("aircast-lite-arm64-{t}.img.xz");
                    release_for(pi, &github(t, false, &[&name, &format!("{name}.sha256")]))
                })
                .collect()
        };
        let versions = |r: Vec<Release>| r.into_iter().map(|r| r.version).collect::<Vec<_>>();
        assert_eq!(
            versions(pick_channel(of(&["v0.2.1-dev.7"]), None)),
            ["v0.2.1-dev.7"]
        );
        assert_eq!(
            versions(pick_channel(
                of(&["v0.2.1-dev.7", "v0.3.5", "v0.4.0-beta.2"]),
                None
            )),
            ["v0.3.5", "v0.4.0-beta.2"]
        );
        assert_eq!(
            versions(pick_channel(
                of(&["v0.2.1-dev.7", "v0.3.5"]),
                Some("development")
            )),
            ["v0.2.1-dev.7"]
        );
    }

    #[test]
    fn the_pi_is_the_default_board_and_its_images_come_from_pi_gen() {
        let pi = board(None).unwrap();
        assert_eq!(
            (pi.id, pi.repo, pi.image),
            ("rpi", PI_REPO, "aircast-lite-arm64")
        );
        assert_eq!(board(Some("rpi")).unwrap(), pi);
        assert!(BOARDS[1..].iter().all(|b| b.repo == BOARD_REPO));
    }

    #[test]
    fn board_ids_are_unique_and_only_the_zero_3_variants_share_an_image() {
        let ids: std::collections::HashSet<_> = BOARDS.iter().map(|b| b.id).collect();
        assert_eq!(ids.len(), BOARDS.len());
        let shared: Vec<_> = BOARDS[1..]
            .iter()
            .filter(|b| b.image != format!("aircast-{}", b.id))
            .map(|b| (b.id, b.image))
            .collect();
        assert_eq!(
            shared,
            [
                ("radxa-zero3w", "aircast-radxa-zero3"),
                ("radxa-zero3e", "aircast-radxa-zero3")
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
    fn a_board_reaches_the_app_in_camel_case_without_where_its_images_live() {
        let json = serde_json::to_value(BOARDS[1]).unwrap();
        assert_eq!(json["builtinWifi"], false);
        assert_eq!(json["maker"], "Orange Pi");
        assert_eq!(json["detail"], "Rockchip RK3588S");
        assert!(json.get("repo").is_none() && json.get("image").is_none());
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
