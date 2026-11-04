use std::io::Write;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::cache::CachedDevice;
use crate::engine::{fat_partition_extent, BlockDevice};

pub const MIN_IMAGE: &str = "0.4.0-beta.4";

const UPDATE_DIR: &str = "aircast-update";
const DEB_PATH: &str = "aircast-update/aircastd.deb";
const SHA_PATH: &str = "aircast-update/aircastd.deb.sha256";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AircastdPackage {
    pub version: String,
    pub filename: String,
    pub sha256: String,
}

impl AircastdPackage {
    pub fn label(&self) -> String {
        match self.version.contains('-') {
            true => format!("{} (beta)", self.version),
            false => self.version.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AircastdUpdate {
    pub version: String,
    pub sha256: String,
    pub deb: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum AircastdOutcome {
    Added(String),
    BuiltIn,
    Skipped(String),
}

impl std::fmt::Display for AircastdOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Added(version) => write!(
                f,
                "{version}, installed on first boot if it's newer than the image's"
            ),
            Self::BuiltIn => f.write_str("the version built into the image"),
            Self::Skipped(why) => write!(
                f,
                "{why} — the card will use the aircastd version built into the image"
            ),
        }
    }
}

pub fn channel(image: &str) -> Option<&'static str> {
    let version = image_version(image)?;
    let min = semver::Version::parse(MIN_IMAGE).expect("MIN_IMAGE is a version");
    (version >= min).then(|| match version.pre.as_str().split('.').next() {
        Some("") | None => "stable",
        Some("dev") => "development",
        Some("stage") => "staging",
        Some(_) => "testing",
    })
}

fn image_version(image: &str) -> Option<semver::Version> {
    let name = image.rsplit(['/', '\\']).next()?;
    let stem = name.split(".img").next()?;
    let version = stem
        .rsplit_once("-v")
        .map_or_else(|| stem.strip_prefix('v'), |(_, version)| Some(version))?;
    semver::Version::parse(version).ok()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

pub fn newest(index: &str) -> Option<AircastdPackage> {
    index
        .lines()
        .collect::<Vec<_>>()
        .split(|line| line.trim().is_empty())
        .filter_map(arm64_aircastd)
        .max_by(|a, b| a.0.cmp(&b.0))
        .map(|(_, package)| package)
}

fn arm64_aircastd(stanza: &[&str]) -> Option<(semver::Version, AircastdPackage)> {
    let field = |name: &str| {
        stanza
            .iter()
            .filter(|line| !line.starts_with([' ', '\t']))
            .filter_map(|line| line.split_once(':'))
            .find(|(key, _)| *key == name)
            .map(|(_, value)| value.trim())
    };
    (field("Package")? == "aircastd" && field("Architecture")? == "arm64").then_some(())?;
    let version = semver::Version::parse(&field("Version")?.replace('~', "-")).ok()?;
    let sha256 = field("SHA256")
        .filter(|s| s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit()))?
        .to_ascii_lowercase();
    let filename = field("Filename")
        .filter(|f| !f.starts_with('/') && !f.split('/').any(|part| part == ".."))?
        .to_string();
    Some((
        version.clone(),
        AircastdPackage {
            version: format!("v{version}"),
            filename,
            sha256,
        },
    ))
}

pub(crate) fn add_to_card(
    device: &mut dyn BlockDevice,
    first_block: &[u8],
    update: &AircastdUpdate,
) -> Result<AircastdOutcome, String> {
    let mut cache = CachedDevice::new(device).map_err(|e| format!("open the card: {e}"))?;
    match stage(&mut cache, first_block, update) {
        Err(reason) => Ok(AircastdOutcome::Skipped(reason)),
        Ok(()) => {
            cache.dirty_mut().for_each(drop_lfn_from_dot_entries);
            cache
                .commit()
                .map(|()| AircastdOutcome::Added(update.version.clone()))
                .map_err(|e| format!("write aircastd to the card: {e}"))
        }
    }
}

const ENTRY: usize = 32;
const DOT: &[u8; 11] = b".          ";
const DOTDOT: &[u8; 11] = b"..         ";
const LFN_ATTR: u8 = 0x0f;
const DELETED: u8 = 0xe5;

pub(crate) fn has_lfn_dot_entries(entries: &[u8]) -> bool {
    entries[11] == LFN_ATTR
        && entries[ENTRY..ENTRY + 11] == *DOT
        && entries[2 * ENTRY + 11] == LFN_ATTR
        && entries[3 * ENTRY..3 * ENTRY + 11] == *DOTDOT
}

fn drop_lfn_from_dot_entries(chunk: &mut [u8]) {
    let found = (0..chunk.len().saturating_sub(4 * ENTRY - 1))
        .step_by(ENTRY)
        .find(|&at| has_lfn_dot_entries(&chunk[at..at + 4 * ENTRY]));
    if let Some(at) = found {
        let entries = &mut chunk[at..at + 4 * ENTRY];
        let dot = entries[ENTRY..2 * ENTRY].to_vec();
        let dotdot: Vec<u8> = entries[3 * ENTRY..4 * ENTRY]
            .iter()
            .enumerate()
            .map(|(i, &b)| match i {
                20 | 21 | 26 | 27 => 0,
                _ => b,
            })
            .collect();
        entries[..ENTRY].copy_from_slice(&dot);
        entries[ENTRY..2 * ENTRY].copy_from_slice(&dotdot);
        entries[2 * ENTRY] = DELETED;
        entries[3 * ENTRY] = DELETED;
    }
}

fn stage(
    cache: &mut CachedDevice<'_>,
    first_block: &[u8],
    update: &AircastdUpdate,
) -> Result<(), String> {
    let deb = std::fs::read(&update.deb)
        .map_err(|e| format!("couldn't read the aircastd download ({e})"))?;
    if sha256_hex(&deb) != update.sha256 {
        return Err("the aircastd download doesn't match its checksum".into());
    }
    let (offset, length) = fat_partition_extent(first_block)
        .map_err(|_| "the image has no boot partition to carry aircastd".to_string())?;
    let unreadable = |e: std::io::Error| format!("couldn't open the card's boot partition ({e})");
    let slice = fscommon::StreamSlice::new(cache, offset, offset + length).map_err(unreadable)?;
    let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new()).map_err(unreadable)?;
    let stats = fs.stats().map_err(unreadable)?;
    let cluster = u64::from(stats.cluster_size());
    let needed = (deb.len() as u64).div_ceil(cluster) + 2;
    let free = u64::from(stats.free_clusters());
    if free < needed {
        return Err(format!(
            "the card's boot partition has no room for aircastd ({:.1} MB free, {:.1} MB needed)",
            (free * cluster) as f64 / 1e6,
            (needed * cluster) as f64 / 1e6
        ));
    }
    let failed = |e: std::io::Error| format!("couldn't write aircastd to the boot partition ({e})");
    let written = {
        let root = fs.root_dir();
        root.create_dir(UPDATE_DIR).and_then(|_| {
            [
                (DEB_PATH, deb.as_slice()),
                (SHA_PATH, format!("{}\n", update.sha256).as_bytes()),
            ]
            .iter()
            .try_for_each(|(path, bytes)| {
                let mut file = root.create_file(path)?;
                file.truncate()?;
                file.write_all(bytes)
            })
        })
    };
    written.map_err(failed)?;
    fs.unmount().map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHA_A: &str = "6585a7b573951977db2353ae00334dcf72ab8e9a7af5eed50891615d9dc23dd7";
    const SHA_B: &str = "84815793641c9cb64ad47511e6102e2857a17dd8e74e530c359c460c20933e44";

    fn stanza(package: &str, arch: &str, version: &str, sha: &str) -> String {
        format!(
            "Package: {package}\n\
             Priority: optional\n\
             Architecture: {arch}\n\
             Version: {version}\n\
             Depends: bash, ca-certificates, systemd\n\
             Filename: pool/main/a/{package}/{package}_{version}_linux_{arch}.deb\n\
             Size: 48730168\n\
             SHA256: {sha}\n\
             Description: aircastd is the on-device control plane:\n \
             Version: 9.9.9\n"
        )
    }

    #[test]
    fn picks_the_newest_arm64_aircastd_from_the_index() {
        let index = [
            stanza("aircastd", "arm64", "0.4.5", SHA_B),
            stanza("aircastd", "arm64", "0.4.6~rc.19", SHA_B),
            stanza("aircastd", "arm64", "0.4.6", SHA_A),
            stanza("aircastd", "arm64", "0.4.6~rc.2", SHA_B),
            stanza("aircastd", "amd64", "0.9.0", SHA_B),
            stanza("aircast-streamer", "arm64", "1.0.0", SHA_B),
        ]
        .join("\n");

        assert_eq!(
            newest(&index),
            Some(AircastdPackage {
                version: "v0.4.6".into(),
                filename: "pool/main/a/aircastd/aircastd_0.4.6_linux_arm64.deb".into(),
                sha256: SHA_A.into(),
            })
        );
    }

    #[test]
    fn release_candidates_order_by_number_and_below_their_release() {
        let index = [
            stanza("aircastd", "arm64", "0.4.6~rc.9", SHA_B),
            stanza("aircastd", "arm64", "0.4.6~rc.19", SHA_A),
        ]
        .join("\r\n");
        assert_eq!(
            newest(&index).map(|p| p.version).as_deref(),
            Some("v0.4.6-rc.19")
        );
    }

    #[test]
    fn an_index_without_a_usable_aircastd_has_nothing_to_offer() {
        assert_eq!(newest(""), None);
        assert_eq!(newest("<html>404 Not Found</html>"), None);
        assert_eq!(newest(&stanza("aircastd", "arm64", "0.4.6", "abc")), None);
        assert_eq!(
            newest(&stanza("aircastd", "arm64", "not-a-version", SHA_A)),
            None
        );
        assert_eq!(
            newest(
                &stanza("aircastd", "arm64", "0.4.6", SHA_A)
                    .replace("Filename: pool/", "Filename: ../../pool/")
            ),
            None
        );
    }

    #[test]
    fn the_checksum_is_lowercase_hex_of_the_bytes() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            newest(&stanza(
                "aircastd",
                "arm64",
                "0.4.6",
                &SHA_A.to_ascii_uppercase()
            ))
            .map(|p| p.sha256),
            Some(SHA_A.to_string())
        );
    }

    #[test]
    fn only_images_from_v0_4_0_beta_4_take_an_aircastd_update() {
        let takes = |image: &str| channel(image).is_some();
        assert!(!takes("v0.3.5"));
        assert!(!takes("v0.4.0-beta.3"));
        assert!(takes("v0.4.0-beta.4"));
        assert!(takes("v0.4.0"));
        assert!(takes("v0.5.1"));
        assert!(!takes("my-own-build.img"));
        assert!(!takes("not-a-version"));
    }

    #[test]
    fn aircastd_follows_the_channel_the_image_was_built_from() {
        assert_eq!(channel("v0.4.0"), Some("stable"));
        assert_eq!(channel("v0.4.0-beta.4"), Some("testing"));
        assert_eq!(channel("v0.4.1-rc.2"), Some("testing"));
        assert_eq!(channel("v0.4.1-dev.3"), Some("development"));
        assert_eq!(channel("v0.4.1-stage.1"), Some("staging"));
    }

    #[test]
    fn the_image_version_comes_from_a_release_or_its_file_name() {
        assert_eq!(
            channel("/cache/images/aircast-lite-arm64-v0.4.0-beta.4.img.xz"),
            Some("testing")
        );
        assert_eq!(
            channel(r"C:\cache\aircast-orangepi5-v0.4.1.img"),
            Some("stable")
        );
        assert_eq!(
            channel("aircast-lite-arm64-v0.4.1-dev.3.img.gz"),
            Some("development")
        );
        assert_eq!(channel("aircast-lite-arm64-v0.3.5.img.xz"), None);
    }

    #[test]
    fn a_beta_aircastd_says_so() {
        let package = |version: &str| AircastdPackage {
            version: version.into(),
            filename: String::new(),
            sha256: String::new(),
        };
        assert_eq!(package("v0.4.6-rc.19").label(), "v0.4.6-rc.19 (beta)");
        assert_eq!(package("v0.4.5").label(), "v0.4.5");
    }

    #[test]
    fn every_outcome_says_which_aircastd_the_card_runs() {
        assert_eq!(
            AircastdOutcome::Skipped("couldn't reach apt.aircast.one".into()).to_string(),
            "couldn't reach apt.aircast.one — the card will use the aircastd version built into the image"
        );
        assert_eq!(
            AircastdOutcome::BuiltIn.to_string(),
            "the version built into the image"
        );
        assert_eq!(
            serde_json::to_string(&AircastdOutcome::Added("v0.4.6".into())).unwrap(),
            r#"{"added":"v0.4.6"}"#
        );
    }
}
