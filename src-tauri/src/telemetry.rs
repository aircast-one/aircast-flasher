use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

use flasher_core::{InitFormat, ProvisionConfig, SshMode};
use serde::Serialize;
use tauri::{AppHandle, Manager};

const MAX_LOG_BYTES: u64 = 2 * 1024 * 1024;
const LOG_FILE: &str = "events.jsonl";

pub fn log_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| format!("No log directory: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create log directory: {e}"))?;
    Ok(dir.join(LOG_FILE))
}

pub fn record<T: Serialize>(app: &AppHandle, event: &T) {
    let Ok(path) = log_path(app) else { return };
    let Ok(line) = serde_json::to_string(event) else {
        return;
    };
    append_line(&path, &line);
}

/// Append one JSONL line, rolling the file over once it passes [`MAX_LOG_BYTES`]
/// so a long-lived install can't grow it without bound. Best-effort: diagnostics
/// must never break a flash.
fn append_line(path: &std::path::Path, line: &str) {
    if std::fs::metadata(path).map(|m| m.len()).unwrap_or(0) > MAX_LOG_BYTES {
        let _ = std::fs::rename(path, path.with_extension("jsonl.1"));
    }

    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(f, "{line}");
    }
}

#[derive(Serialize, Clone, Copy)]
pub struct ConfigFacts {
    pub wifi: bool,
    pub hostname: bool,
    pub remote: &'static str,
    pub ssh: &'static str,
    pub init_format: &'static str,
}

pub fn describe_config(cfg: &ProvisionConfig) -> ConfigFacts {
    ConfigFacts {
        wifi: cfg.wifi.is_some(),
        hostname: cfg.hostname.as_deref().is_some_and(|h| !h.trim().is_empty()),
        remote: match cfg.tailscale.as_ref() {
            None => "off",
            Some(ts) if ts.control_server.trim().is_empty() => "tailscale",
            Some(_) => "headscale",
        },
        ssh: match cfg.access.as_ref().map(|a| a.ssh) {
            None => "image-default",
            Some(SshMode::KeyOnly) => "key",
            Some(SshMode::Password) => "password",
            Some(SshMode::Disabled) => "disabled",
        },
        init_format: match cfg.init_format {
            InitFormat::CloudInit => "cloud-init",
            InitFormat::FirstRun => "first-run",
        },
    }
}

#[derive(Serialize)]
pub struct Envelope {
    pub event: &'static str,
    pub job_id: String,
    pub app_version: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub outcome: &'static str,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl Envelope {
    pub fn new(
        event: &'static str,
        job_id: String,
        app: &AppHandle,
        outcome: &'static str,
        duration_ms: u64,
        error: Option<String>,
    ) -> Self {
        Self {
            event,
            job_id,
            app_version: app.package_info().version.to_string(),
            os: std::env::consts::OS,
            arch: std::env::consts::ARCH,
            outcome,
            duration_ms,
            error,
        }
    }
}

#[derive(Serialize)]
pub struct DownloadEvent {
    #[serde(flatten)]
    pub envelope: Envelope,
    pub bytes: u64,
    pub cached: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mbps: Option<f64>,
}

#[derive(Serialize)]
pub struct FlashEvent {
    #[serde(flatten)]
    pub envelope: Envelope,
    #[serde(flatten)]
    pub config: ConfigFacts,
    pub image_bytes: u64,
    pub compressed: bool,
    pub verified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed_at: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decompress_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verify_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub customize_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub write_mbps: Option<f64>,
}

/// Stage transitions observed on the helper's progress stream, in order. Phase
/// durations are the gaps between transitions, so they need no cooperation from
/// the elevated process.
#[derive(Default)]
pub struct PhaseLog {
    transitions: Vec<(&'static str, Instant)>,
    pub bytes: u64,
}

impl PhaseLog {
    pub fn mark(&mut self, stage: &'static str, bytes: u64) {
        self.bytes = self.bytes.max(bytes);
        if self.transitions.last().map(|(s, _)| *s) != Some(stage) {
            self.transitions.push((stage, Instant::now()));
        }
    }

    pub fn last_stage(&self) -> Option<&'static str> {
        self.transitions.last().map(|(s, _)| *s)
    }

    pub fn saw(&self, stage: &str) -> bool {
        self.transitions.iter().any(|(s, _)| *s == stage)
    }

    pub fn duration_ms(&self, stage: &str, end: Instant) -> Option<u64> {
        let start = self
            .transitions
            .iter()
            .position(|(s, _)| *s == stage)
            .map(|i| self.transitions[i].1)?;
        let next = self
            .transitions
            .iter()
            .skip_while(|(s, _)| *s != stage)
            .find(|(s, _)| *s != stage)
            .map(|(_, t)| *t)
            .unwrap_or(end);
        Some(next.duration_since(start).as_millis() as u64)
    }
}

pub fn mbps(bytes: u64, ms: u64) -> Option<f64> {
    (ms > 0).then(|| (bytes as f64 / 1_000_000.0) / (ms as f64 / 1000.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use flasher_core::{AccessConfig, TailscaleConfig, WifiConfig};

    fn config_with_secrets() -> ProvisionConfig {
        ProvisionConfig {
            hostname: Some("falcon-01".into()),
            wifi: Some(WifiConfig {
                ssid: "secret-field-net".into(),
                password: "wifi-passphrase".into(),
                country: "UA".into(),
            }),
            tailscale: Some(TailscaleConfig {
                control_server: "https://hs.example.com".into(),
                auth_key: "tskey-auth-supersecret".into(),
            }),
            access: Some(AccessConfig {
                ssh: SshMode::Password,
                password: Some("device-password".into()),
                authorized_key: None,
            }),
            init_format: InitFormat::CloudInit,
        }
    }

    #[test]
    fn config_facts_carry_no_secrets_or_identifiers() {
        let facts = describe_config(&config_with_secrets());
        let json = serde_json::to_string(&facts).expect("serialize");

        for secret in [
            "secret-field-net",
            "wifi-passphrase",
            "tskey-auth-supersecret",
            "device-password",
            "falcon-01",
            "hs.example.com",
        ] {
            assert!(!json.contains(secret), "event leaked {secret}: {json}");
        }

        assert!(json.contains("\"wifi\":true"));
        assert!(json.contains("\"hostname\":true"));
        assert!(json.contains("\"remote\":\"headscale\""));
        assert!(json.contains("\"ssh\":\"password\""));
    }

    #[test]
    fn config_facts_describe_the_absent_case() {
        let facts = describe_config(&ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: None,
            access: None,
            init_format: InitFormat::FirstRun,
        });
        let json = serde_json::to_string(&facts).expect("serialize");
        assert!(json.contains("\"wifi\":false"));
        assert!(json.contains("\"remote\":\"off\""));
        assert!(json.contains("\"ssh\":\"image-default\""));
        assert!(json.contains("\"init_format\":\"first-run\""));
    }

    #[test]
    fn blank_hostname_is_not_a_hostname() {
        let facts = describe_config(&ProvisionConfig {
            hostname: Some("   ".into()),
            wifi: None,
            tailscale: None,
            access: None,
            init_format: InitFormat::CloudInit,
        });
        assert!(!facts.hostname);
    }

    #[test]
    fn tailscale_without_a_control_server_is_the_hosted_service() {
        let facts = describe_config(&ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: Some(TailscaleConfig {
                control_server: "  ".into(),
                auth_key: "tskey-auth-x".into(),
            }),
            access: None,
            init_format: InitFormat::CloudInit,
        });
        assert_eq!(facts.remote, "tailscale");
    }

    #[test]
    fn phase_durations_come_from_the_gaps_between_transitions() {
        let mut log = PhaseLog::default();
        log.mark("write", 10);
        let write_start = log.transitions[0].1;
        log.mark("write", 4096);
        log.transitions.push(("verify", write_start + ms(500)));
        log.transitions.push(("customize", write_start + ms(700)));
        let end = write_start + ms(750);

        assert_eq!(log.duration_ms("write", end), Some(500));
        assert_eq!(log.duration_ms("verify", end), Some(200));
        assert_eq!(log.duration_ms("customize", end), Some(50));
        assert_eq!(log.duration_ms("download", end), None);
        assert_eq!(log.bytes, 4096);
        assert_eq!(log.last_stage(), Some("customize"));
        assert!(log.saw("verify"));
    }

    #[test]
    fn a_repeated_stage_is_one_transition() {
        let mut log = PhaseLog::default();
        log.mark("write", 1);
        log.mark("write", 2);
        log.mark("write", 3);
        assert_eq!(log.transitions.len(), 1);
    }

    #[test]
    fn throughput_needs_elapsed_time() {
        assert_eq!(mbps(0, 0), None);
        assert_eq!(mbps(2_000_000, 1000), Some(2.0));
    }

    fn ms(n: u64) -> std::time::Duration {
        std::time::Duration::from_millis(n)
    }

    fn temp_log(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aircast-telemetry-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join(LOG_FILE)
    }

    #[test]
    fn events_append_as_one_json_object_per_line() {
        let path = temp_log("append");
        append_line(&path, r#"{"event":"flash","outcome":"success"}"#);
        append_line(&path, r#"{"event":"download","outcome":"failed"}"#);

        let contents = std::fs::read_to_string(&path).expect("read log");
        let events: Vec<serde_json::Value> = contents
            .lines()
            .map(|l| serde_json::from_str(l).expect("each line is valid JSON"))
            .collect();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["event"], "download");
    }

    #[test]
    fn an_oversized_log_rolls_over_instead_of_growing() {
        let path = temp_log("rollover");
        std::fs::write(&path, vec![b'x'; (MAX_LOG_BYTES + 1) as usize]).expect("seed big log");

        append_line(&path, r#"{"event":"flash"}"#);

        let current = std::fs::read_to_string(&path).expect("read log");
        assert_eq!(current.lines().count(), 1, "current log restarts");
        assert!(path.with_extension("jsonl.1").exists(), "previous log kept");
    }

    #[test]
    fn a_flash_event_serializes_as_one_flat_object() {
        let event = FlashEvent {
            envelope: Envelope {
                event: "flash",
                job_id: "job-1".into(),
                app_version: "0.1.1".into(),
                os: "macos",
                arch: "aarch64",
                outcome: "failed",
                duration_ms: 91_000,
                error: Some("write to device: Input/output error".into()),
            },
            config: describe_config(&config_with_secrets()),
            image_bytes: 3_800_000_000,
            compressed: true,
            verified: false,
            failed_at: Some("write"),
            decompress_ms: Some(21_000),
            write_ms: Some(70_000),
            verify_ms: None,
            customize_ms: None,
            write_mbps: Some(54.3),
        };

        let v: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&event).expect("serialize"))
                .expect("valid json");

        assert!(v.is_object());
        assert_eq!(v["event"], "flash");
        assert_eq!(v["failed_at"], "write");
        assert_eq!(v["ssh"], "password", "config facts are flattened, not nested");
        assert_eq!(v["job_id"], "job-1", "envelope is flattened, not nested");
        assert!(v.get("verify_ms").is_none(), "phases that never ran are omitted");
        assert!(
            !serde_json::to_string(&event)
                .expect("serialize")
                .contains("device-password"),
            "no secrets in the wide event"
        );
    }

    #[test]
    fn recording_survives_an_unwritable_path() {
        append_line(
            std::path::Path::new("/nonexistent-dir-aircast/events.jsonl"),
            r#"{"event":"flash"}"#,
        );
    }
}
