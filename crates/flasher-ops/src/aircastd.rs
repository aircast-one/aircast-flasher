use std::path::{Path, PathBuf};

use flasher_core::aircastd::{channel, sha256_hex};
use flasher_core::{AircastdOutcome, AircastdPackage, AircastdUpdate};

use super::http;

const APT: &str = "https://apt.aircast.one/aircastd";

pub async fn newest(image: &str) -> Result<Option<AircastdPackage>, String> {
    let Some(channel) = channel(image) else {
        return Ok(None);
    };
    let response = http()
        .get(format!("{APT}/dists/{channel}/main/binary-arm64/Packages"))
        .send()
        .await
        .map_err(|_| "couldn't reach apt.aircast.one".to_string())?;
    if !response.status().is_success() {
        return Err(format!(
            "apt.aircast.one has no {channel} aircastd (HTTP {})",
            response.status().as_u16()
        ));
    }
    let index = response
        .text()
        .await
        .map_err(|_| format!("couldn't read the {channel} aircastd index"))?;
    flasher_core::aircastd::newest(&index)
        .map(Some)
        .ok_or_else(|| format!("apt.aircast.one has no arm64 aircastd in {channel}"))
}

pub async fn check(image: &str) -> AircastdOutcome {
    match newest(image).await {
        Ok(Some(package)) => AircastdOutcome::Added(package.label()),
        Ok(None) => AircastdOutcome::BuiltIn,
        Err(why) => AircastdOutcome::Skipped(why),
    }
}

pub async fn prepare(image: &str) -> Result<AircastdUpdate, AircastdOutcome> {
    let package = newest(image)
        .await
        .map_err(AircastdOutcome::Skipped)?
        .ok_or(AircastdOutcome::BuiltIn)?;
    let dir = crate::paths::cache_dir()
        .map_err(AircastdOutcome::Skipped)?
        .join("aircastd");
    let deb = fetch(&dir, &package)
        .await
        .map_err(AircastdOutcome::Skipped)?;
    Ok(AircastdUpdate {
        version: package.label(),
        sha256: package.sha256,
        deb: deb.to_string_lossy().into_owned(),
    })
}

async fn fetch(dir: &Path, package: &AircastdPackage) -> Result<PathBuf, String> {
    let deb = dir.join(format!(
        "aircastd_{}_{}.deb",
        package.version,
        &package.sha256[..16]
    ));
    if deb.is_file() {
        return Ok(deb);
    }
    let failed = || format!("couldn't download aircastd {}", package.version);
    let bytes = http()
        .get(format!("{APT}/{}", package.filename))
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|_| failed())?
        .bytes()
        .await
        .map_err(|_| failed())?;
    if sha256_hex(&bytes) != package.sha256 {
        return Err(format!(
            "the aircastd {} download didn't match its checksum",
            package.version
        ));
    }
    let part = deb.with_extension(format!("{}.part", crate::random_hex(4)));
    std::fs::create_dir_all(dir)
        .and_then(|()| std::fs::write(&part, &bytes))
        .and_then(|()| std::fs::rename(&part, &deb))
        .map_err(|e| format!("couldn't keep the aircastd download ({e})"))?;
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path != &deb && path.extension().is_some_and(|ext| ext == "deb"))
        .for_each(|path| {
            let _ = std::fs::remove_file(path);
        });
    Ok(deb)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_image_older_than_the_first_boot_install_keeps_its_own_aircastd() {
        assert_eq!(check("v0.3.5").await, AircastdOutcome::BuiltIn);
        assert_eq!(
            prepare("aircast-lite-arm64-v0.4.0-beta.3.img.xz").await,
            Err(AircastdOutcome::BuiltIn)
        );
    }

    #[tokio::test]
    async fn a_cached_download_is_reused_without_the_network() {
        let dir = crate::isolated_dir("aircastd-cache");
        let package = AircastdPackage {
            version: "v0.4.6-rc.19".into(),
            filename: "pool/never-fetched.deb".into(),
            sha256: "6585a7b573951977db2353ae00334dcf72ab8e9a7af5eed50891615d9dc23dd7".into(),
        };
        let cached = dir.join("aircastd_v0.4.6-rc.19_6585a7b573951977.deb");
        std::fs::write(&cached, b"deb").unwrap();

        assert_eq!(fetch(&dir, &package).await, Ok(cached));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
