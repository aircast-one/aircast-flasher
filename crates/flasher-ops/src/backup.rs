use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::SystemTime;

use chrono::{DateTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

const FORMAT: u64 = 1;
const NOT_A_BACKUP: &str = "This isn't an Aircast backup";
const KEEP_PER_HOST: usize = 5;
const FRESH_HOURS: i64 = 24;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Backup {
    pub id: String,
    pub hostname: String,
    pub created: String,
    pub card: Option<Card>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Card {
    pub fight_mode: bool,
    pub password_login: bool,
    pub encryption: Option<CardEncryption>,
    pub control_server: String,
    pub tailnet_node: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CardEncryption {
    pub threshold: u32,
    pub tang_urls: Vec<String>,
    #[serde(skip_deserializing)]
    pub cloud: bool,
}

impl Card {
    fn split_cloud(self, cloud_prefix: &str) -> Self {
        Card {
            encryption: self.encryption.map(|e| e.split_cloud(cloud_prefix)),
            ..self
        }
    }
}

impl CardEncryption {
    fn split_cloud(self, cloud_prefix: &str) -> Self {
        let (cloud, own): (Vec<String>, Vec<String>) = self
            .tang_urls
            .into_iter()
            .partition(|url| url.trim().starts_with(cloud_prefix));
        let servers = own.len() as u32 + u32::from(!cloud.is_empty());
        CardEncryption {
            threshold: self.threshold.min(servers),
            tang_urls: own,
            cloud: !cloud.is_empty(),
        }
    }

    pub fn servers_known(&self) -> bool {
        self.threshold > 0
    }
}

#[derive(Deserialize)]
struct Header {
    #[serde(default)]
    format: Option<u64>,
    #[serde(default)]
    hostname: String,
    #[serde(default)]
    created: String,
    #[serde(default)]
    card: Option<serde_json::Value>,
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
        card: header
            .card
            .and_then(|card| serde_json::from_value::<Card>(card).ok())
            .map(|card| card.split_cloud(&crate::cloud::tang_prefix())),
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
    let saved = summary(id, header);
    prune(dir, &saved.hostname);
    Ok(saved)
}

fn prune(dir: &Path, hostname: &str) {
    list(dir)
        .into_iter()
        .filter(|backup| backup.hostname == hostname)
        .skip(KEEP_PER_HOST)
        .for_each(|backup| {
            let _ = delete(dir, &backup.id);
        });
}

fn created_at(backup: &Backup) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&backup.created)
        .ok()
        .map(|at| at.with_timezone(&Utc))
}

pub fn age(backup: &Backup, now: SystemTime) -> Option<std::time::Duration> {
    (DateTime::<Utc>::from(now) - created_at(backup)?)
        .to_std()
        .ok()
}

fn due(backups: &[Backup], hostname: &str, now: SystemTime) -> bool {
    let now = DateTime::<Utc>::from(now);
    !backups
        .iter()
        .filter(|backup| backup.hostname.eq_ignore_ascii_case(hostname))
        .filter_map(created_at)
        .any(|at| now - at < TimeDelta::hours(FRESH_HOURS))
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

#[derive(Default)]
pub struct InFlight(Mutex<HashSet<String>>);

struct Claim<'a> {
    in_flight: &'a InFlight,
    url: String,
}

impl InFlight {
    fn claim(&self, url: &str) -> Option<Claim<'_>> {
        self.0.lock().ok()?.insert(url.to_string()).then(|| Claim {
            in_flight: self,
            url: url.to_string(),
        })
    }
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if let Ok(mut urls) = self.in_flight.0.lock() {
            urls.remove(&self.url);
        }
    }
}

pub async fn take_once(dir: &Path, url: &str, in_flight: &InFlight) -> Result<Backup, String> {
    let _claim = in_flight.claim(url).ok_or_else(|| {
        "This drone is already being backed up — wait for it to finish".to_string()
    })?;
    take(dir, url).await
}

struct Drone {
    hostname: String,
    url: String,
}

async fn back_up_due(
    dir: &Path,
    drones: Vec<Drone>,
    in_flight: &InFlight,
    now: SystemTime,
) -> Vec<Backup> {
    let backups = list(dir);
    futures::future::join_all(
        drones
            .into_iter()
            .filter(|drone| due(&backups, &drone.hostname, now))
            .map(|drone| async move { take_once(dir, &drone.url, in_flight).await.ok() }),
    )
    .await
    .into_iter()
    .flatten()
    .collect()
}

pub async fn back_up_stale_drones(dir: &Path, in_flight: &InFlight) -> Vec<Backup> {
    if cfg!(feature = "no-real-devices") {
        return Vec::new();
    }
    let url = |peer: &crate::tailnet::TailnetPeer| format!("http://{}", peer.dns_name);
    let online: Vec<_> = crate::tailnet::local_tailscale()
        .await
        .peers
        .into_iter()
        .filter(|peer| peer.online && peer.os.eq_ignore_ascii_case("linux"))
        .collect();
    let answering = crate::tailnet::probe_devices(online.iter().map(url).collect()).await;
    let drones = online
        .into_iter()
        .filter(|peer| answering.contains(&url(peer)))
        .map(|peer| Drone {
            url: url(&peer),
            hostname: peer.host_name,
        })
        .collect();
    back_up_due(dir, drones, in_flight, SystemTime::now()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::isolated_dir;

    const BUNDLE: &str = r#"{"format":1,"hostname":"falcon","created":"2026-11-01T12:00:00Z","files":[{"path":"/var/lib/aircastd/gimbal.json","mode":384,"data":"eyJ0aWx0IjoxfQ=="}]}"#;

    fn json(s: &str) -> serde_json::Value {
        serde_json::from_str(s).unwrap()
    }

    const CARD: &str = r#""card":{"fightMode":true,"passwordLogin":false,"encryption":{"threshold":1,"tangUrls":["https://tang.example.com","https://api.aircast.one/v1/tang/3f2a"]},"controlServer":"https://hs.example.com","tailnetNode":true}"#;

    fn with_card(card: &str) -> String {
        BUNDLE.replacen('{', &format!("{{{card},"), 1)
    }

    fn bundle(hostname: &str, created: &str) -> String {
        BUNDLE
            .replace(
                r#""hostname":"falcon""#,
                &format!(r#""hostname":"{hostname}""#),
            )
            .replace("2026-11-01T12:00:00Z", created)
    }

    fn at(rfc3339: &str) -> SystemTime {
        DateTime::parse_from_rfc3339(rfc3339).unwrap().into()
    }

    #[test]
    fn the_card_is_read_and_a_missing_or_broken_one_is_unknown() {
        let dir = isolated_dir("backup-card");
        let card = save(&dir, with_card(CARD).as_bytes())
            .unwrap()
            .card
            .unwrap();
        assert!(card.fight_mode && card.tailnet_node && !card.password_login);
        assert_eq!(card.control_server, "https://hs.example.com");
        assert!(card
            .encryption
            .as_ref()
            .unwrap()
            .tang_urls
            .contains(&"https://tang.example.com".to_string()));
        assert_eq!(
            get(&dir, "falcon-20261101120000")
                .unwrap()
                .card
                .map(|c| c.control_server),
            Some(card.control_server)
        );

        [
            BUNDLE.to_string(),
            with_card(r#""card":null"#),
            with_card(r#""card":"garbage""#),
        ]
        .iter()
        .for_each(|bundle| assert_eq!(save(&dir, bundle.as_bytes()).unwrap().card, None));
        let plain = with_card(r#""card":{"encryption":null}"#);
        assert_eq!(
            save(&dir, plain.as_bytes()).unwrap().card,
            Some(Card::default())
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cloud_key_servers_become_the_cloud_option_and_the_rest_stay_self_hosted() {
        let prefix = "https://api.aircast.one/v1/tang/";
        let split = |threshold: u32, urls: &[&str]| {
            CardEncryption {
                threshold,
                tang_urls: urls.iter().map(|url| url.to_string()).collect(),
                cloud: false,
            }
            .split_cloud(prefix)
        };
        assert_eq!(
            split(
                2,
                &[
                    "https://tang.example.com",
                    "https://api.aircast.one/v1/tang/3f2a"
                ]
            ),
            CardEncryption {
                threshold: 2,
                tang_urls: vec!["https://tang.example.com".into()],
                cloud: true,
            }
        );
        assert_eq!(
            split(1, &["http://10.0.0.5:7500"]),
            CardEncryption {
                threshold: 1,
                tang_urls: vec!["http://10.0.0.5:7500".into()],
                cloud: false,
            }
        );
        assert_eq!(
            split(
                2,
                &[
                    "https://api.aircast.one/v1/tang/a",
                    "https://api.aircast.one/v1/tang/b"
                ]
            ),
            CardEncryption {
                threshold: 1,
                tang_urls: vec![],
                cloud: true,
            }
        );
        assert!(!split(1, &["https://api.aircast.one.evil.example/v1/tang/a"]).cloud);
        assert!(!split(0, &[]).servers_known());
    }

    #[test]
    fn only_the_newest_five_per_drone_are_kept() {
        let dir = isolated_dir("backup-retention");
        save(&dir, bundle("owel", "2026-10-01T00:00:00Z").as_bytes()).unwrap();
        (1..=7).for_each(|day| {
            save(
                &dir,
                bundle("falcon", &format!("2026-11-0{day}T12:00:00Z")).as_bytes(),
            )
            .unwrap();
        });
        let kept: Vec<String> = list(&dir).into_iter().map(|b| b.id).collect();
        assert_eq!(
            kept,
            [
                "falcon-20261107120000",
                "falcon-20261106120000",
                "falcon-20261105120000",
                "falcon-20261104120000",
                "falcon-20261103120000",
                "owel-20261001000000",
            ]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_drone_is_due_once_its_newest_backup_is_a_day_old() {
        let falcon = Backup {
            id: "falcon-20261101120000".into(),
            hostname: "falcon".into(),
            created: "2026-11-01T12:00:00Z".into(),
            card: None,
        };
        let backups = std::slice::from_ref(&falcon);
        assert!(!due(backups, "falcon", at("2026-11-02T11:59:00Z")));
        assert!(!due(backups, "Falcon", at("2026-11-01T13:00:00Z")));
        assert!(due(backups, "falcon", at("2026-11-02T12:00:00Z")));
        assert!(due(backups, "owel", at("2026-11-01T13:00:00Z")));
        let undated = Backup {
            created: "yesterday".into(),
            ..falcon.clone()
        };
        assert!(due(&[undated], "falcon", at("2026-11-01T13:00:00Z")));
        assert_eq!(
            age(&falcon, at("2026-11-01T15:00:00Z")),
            Some(std::time::Duration::from_secs(3 * 3600))
        );
    }

    #[tokio::test]
    async fn a_drone_is_backed_up_once_at_a_time() {
        let in_flight = InFlight::default();
        let claim = in_flight.claim("http://falcon").unwrap();
        assert!(in_flight.claim("http://falcon").is_none());
        assert!(in_flight.claim("http://owel").is_some());
        let dir = isolated_dir("backup-in-flight");
        assert!(take_once(&dir, "http://falcon", &in_flight)
            .await
            .unwrap_err()
            .contains("already being backed up"));
        drop(claim);
        assert!(in_flight.claim("http://falcon").is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn the_automatic_pass_backs_up_only_drones_that_are_due() {
        let dir = isolated_dir("backup-auto");
        save(&dir, bundle("owel", "2026-11-01T11:00:00Z").as_bytes()).unwrap();
        let falcon = answer_once(ok(BUNDLE)).await;
        let owel = answer_once(ok(&bundle("owel", "2026-11-01T12:00:00Z"))).await;
        let old = answer_once("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".into()).await;
        let drone = |hostname: &str, url: &str| Drone {
            hostname: hostname.into(),
            url: url.into(),
        };
        let saved = back_up_due(
            &dir,
            vec![
                drone("falcon", &falcon),
                drone("owel", &owel),
                drone("racer", &old),
                drone("gone", "http://127.0.0.1:1"),
            ],
            &InFlight::default(),
            at("2026-11-01T12:30:00Z"),
        )
        .await;
        assert_eq!(
            saved
                .iter()
                .map(|b| b.hostname.as_str())
                .collect::<Vec<_>>(),
            ["falcon"]
        );
        assert_eq!(list(&dir).len(), 2);
        let _ = std::fs::remove_dir_all(&dir);
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

    fn ok(body: &str) -> String {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[tokio::test]
    async fn a_backup_is_taken_over_http_and_stored_as_sent() {
        let url = answer_once(ok(BUNDLE)).await;
        let dir = isolated_dir("backup-take");
        let saved = take(&dir, &url).await.unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(format!("{}.json", saved.id))).unwrap(),
            BUNDLE
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
