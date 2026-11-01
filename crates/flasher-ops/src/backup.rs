use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FORMAT: u64 = 1;
const NOT_A_BACKUP: &str = "This isn't an Aircast backup";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub hostname: String,
    pub created: String,
}

#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    format: Option<u64>,
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    created: String,
}

fn header(bytes: &[u8]) -> Result<Header, String> {
    let header: Header = serde_json::from_slice(bytes).map_err(|_| NOT_A_BACKUP.to_string())?;
    match header.format {
        Some(FORMAT) => Ok(header),
        Some(other) => Err(format!(
            "This backup is format {other} and this Flasher reads only format {FORMAT} — update the Flasher"
        )),
        None => Err(format!("{NOT_A_BACKUP} — it names no format")),
    }
}

fn summary(id: String, header: Header) -> Backup {
    Backup {
        id,
        hostname: header.hostname,
        created: header.created,
    }
}

fn new_id(header: &Header) -> String {
    let host: String = header
        .hostname
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    let host = match host.trim_matches('-') {
        "" => "drone",
        name => name,
    };
    let stamp: String = header
        .created
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let stamp = match stamp.as_str() {
        "" => std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs().to_string())
            .unwrap_or_default(),
        digits => digits.to_string(),
    };
    format!("{host}-{stamp}")
}

fn path_of(dir: &Path, id: &str) -> Result<PathBuf, String> {
    let safe = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    safe.then(|| dir.join(format!("{id}.json")))
        .ok_or_else(|| format!("No backup named {id} on this computer"))
}

fn read(dir: &Path, id: &str) -> Result<Vec<u8>, String> {
    std::fs::read(path_of(dir, id)?).map_err(|_| format!("No backup named {id} on this computer"))
}

pub fn save(dir: &Path, bytes: &[u8]) -> Result<Backup, String> {
    let header = header(bytes)?;
    let id = new_id(&header);
    std::fs::create_dir_all(dir).map_err(|e| format!("Failed to create {}: {e}", dir.display()))?;
    crate::flash::write_secret_file(&dir.join(format!("{id}.json")), bytes)?;
    Ok(summary(id, header))
}

pub fn list(dir: &Path) -> Vec<Backup> {
    let entries = std::fs::read_dir(dir)
        .map(|entries| entries.filter_map(Result::ok).collect::<Vec<_>>())
        .unwrap_or_default();
    let mut backups: Vec<Backup> = entries
        .iter()
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "json").then_some(())?;
            let id = path.file_stem()?.to_str()?.to_string();
            let header = header(&std::fs::read(&path).ok()?).ok()?;
            Some(summary(id, header))
        })
        .collect();
    backups.sort_by(|a, b| b.created.cmp(&a.created).then_with(|| a.id.cmp(&b.id)));
    backups
}

pub fn get(dir: &Path, id: &str) -> Result<Backup, String> {
    header(&read(dir, id)?).map(|header| summary(id.to_string(), header))
}

pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    std::fs::remove_file(path_of(dir, id)?)
        .map_err(|_| format!("No backup named {id} on this computer"))
}

pub fn restore_contents(bytes: &[u8], same_drone: bool) -> Result<String, String> {
    header(bytes)?;
    let bundle: serde_json::Map<String, serde_json::Value> =
        serde_json::from_slice(bytes).map_err(|_| NOT_A_BACKUP.to_string())?;
    let marked: serde_json::Map<String, serde_json::Value> = bundle
        .into_iter()
        .chain([("sameDrone".to_string(), same_drone.into())])
        .collect();
    serde_json::to_string(&marked).map_err(|e| e.to_string())
}

pub fn restore_file(dir: &Path, id: &str, same_drone: bool) -> Result<String, String> {
    restore_contents(&read(dir, id)?, same_drone)
}

pub async fn fetch(url: &str) -> Result<Vec<u8>, String> {
    let response = crate::http()
        .get(format!("{url}/api/backup"))
        .send()
        .await
        .map_err(|_| {
            format!("Couldn't reach {url} — check the drone is powered on and on your tailnet")
        })?;
    match response.status() {
        status if status.is_success() => response
            .bytes()
            .await
            .map(|body| body.to_vec())
            .map_err(|_| "The drone stopped answering mid-backup — try again".to_string()),
        reqwest::StatusCode::NOT_FOUND => Err(
            "This drone's aircastd can't make backups — update aircastd on the drone first"
                .to_string(),
        ),
        status => Err(format!(
            "The drone refused the backup (HTTP {})",
            status.as_u16()
        )),
    }
}

pub async fn take(dir: &Path, url: &str) -> Result<Backup, String> {
    save(dir, &fetch(url).await?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolated_dir;

    const BUNDLE: &str = r#"{"format":1,"hostname":"falcon","created":"2026-11-01T12:00:00Z","files":[{"path":"/var/lib/aircastd/gimbal.json","mode":384,"data":"eyJ0aWx0IjoxfQ=="}]}"#;

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    #[test]
    fn an_unknown_format_is_refused_and_says_why() {
        let newer = BUNDLE.replace(r#""format":1"#, r#""format":2"#);
        let dir = isolated_dir("backup-format");

        let refused = save(&dir, newer.as_bytes()).unwrap_err();
        assert!(refused.contains("format 2"), "{refused}");
        assert!(list(&dir).is_empty());
        assert!(restore_contents(newer.as_bytes(), true).is_err());
        assert!(restore_contents(br#"{"files":[]}"#, true)
            .unwrap_err()
            .contains("no format"));
        assert_eq!(restore_contents(b"<html>", true).unwrap_err(), NOT_A_BACKUP);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restoring_flags_the_drone_and_leaves_the_files_alone() {
        let original = json(BUNDLE);
        [true, false].into_iter().for_each(|same| {
            let restored = json(&restore_contents(BUNDLE.as_bytes(), same).unwrap());
            assert_eq!(restored["sameDrone"], same);
            assert_eq!(restored["files"], original["files"]);
            assert_eq!(restored["hostname"], "falcon");
            assert_eq!(restored["format"], 1);
        });
    }

    #[test]
    fn a_saved_backup_is_listed_restored_and_deleted() {
        let dir = isolated_dir("backup-store");
        let saved = save(&dir, BUNDLE.as_bytes()).unwrap();
        assert_eq!(saved.id, "falcon-20261101120000");
        assert_eq!(list(&dir), std::slice::from_ref(&saved));
        assert_eq!(get(&dir, &saved.id).unwrap(), saved);
        assert_eq!(
            json(&restore_file(&dir, &saved.id, false).unwrap())["sameDrone"],
            false
        );

        delete(&dir, &saved.id).unwrap();
        assert!(list(&dir).is_empty());
        assert!(restore_file(&dir, &saved.id, true).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_saved_backup_is_owner_only() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = isolated_dir("backup-mode");
        let saved = save(&dir, BUNDLE.as_bytes()).unwrap();
        let mode = std::fs::metadata(dir.join(format!("{}.json", saved.id)))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "a backup holds secrets");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_id_never_reaches_outside_the_backup_dir() {
        let dir = isolated_dir("backup-escape");
        assert!(delete(&dir, "../settings").is_err());
        assert!(get(&dir, "/etc/passwd").is_err());
        assert!(path_of(&dir, "").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_hostile_hostname_still_makes_a_plain_id() {
        let bundle = BUNDLE.replace(r#""hostname":"falcon""#, r#""hostname":"../Evil Host/""#);
        let dir = isolated_dir("backup-hostile");
        let saved = save(&dir, bundle.as_bytes()).unwrap();
        assert_eq!(saved.id, "evilhost-20261101120000");
        assert_eq!(saved.hostname, "../Evil Host/");
        let _ = std::fs::remove_dir_all(&dir);
    }

    async fn answer_once(response: String) -> String {
        use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let _ = socket.read(&mut request).await;
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        url
    }

    #[tokio::test]
    async fn a_drone_without_the_endpoint_is_told_to_update_aircastd() {
        let url = answer_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into()).await;
        assert!(fetch(&url)
            .await
            .unwrap_err()
            .contains("update aircastd on the drone first"));
    }

    #[tokio::test]
    async fn an_unreachable_drone_says_so() {
        assert!(fetch("http://127.0.0.1:1")
            .await
            .unwrap_err()
            .starts_with("Couldn't reach http://127.0.0.1:1"));
    }

    #[tokio::test]
    async fn a_backup_is_taken_over_http_and_stored_as_sent() {
        let url = answer_once(format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{BUNDLE}",
            BUNDLE.len()
        ))
        .await;
        let dir = isolated_dir("backup-take");
        let saved = take(&dir, &url).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("{}.json", saved.id))).unwrap(),
            BUNDLE
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
