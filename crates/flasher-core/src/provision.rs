//! Turn a [`ProvisionConfig`] into the concrete first-boot files to write onto
//! the FAT boot partition. Pure: no I/O. The Tauri app mounts the partition and
//! applies the returned [`ProvisionPlan`].

use crate::{psk, AccessConfig, InitFormat, ProvisionConfig, SshMode, TailscaleConfig, WifiConfig};

const TS_STATE_PATH: &str = "/var/lib/aircastd/tailscale.json";
const TS_KEY_PATH: &str = "/var/lib/aircastd/tailscale.authkey";

/// Sanitized (control_server, auth_key) when a key is present, else `None`.
/// Strips control characters so a pasted value with an embedded newline can't
/// break out of the cloud-init YAML block scalar or the firstrun heredoc.
fn provisioned_tailscale(ts: &TailscaleConfig) -> Option<(String, String)> {
    let control = sanitize_line(&ts.control_server);
    let key = sanitize_line(&ts.auth_key);
    if key.is_empty() {
        return None;
    }
    Some((control, key))
}

fn sanitize_line(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).collect()
}

fn tailscale_state_json(control_server: &str) -> String {
    format!("{{\"loginServer\":{}}}", json_string(control_server))
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A file to write into the boot partition's root.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionFile {
    pub name: String,
    pub contents: String,
    /// Whether the file should be made executable (firstrun.sh).
    pub executable: bool,
}

impl ProvisionFile {
    fn plain(name: &str, contents: String) -> Self {
        Self { name: name.into(), contents, executable: false }
    }
}

/// The kernel-command-line fragment that runs `firstrun.sh` once on first boot.
pub const FIRSTRUN_CMDLINE: &str =
    " systemd.run=/boot/firstrun.sh systemd.run_success_action=reboot systemd.unit=kernel-command-line.target";

/// What to write to the boot partition for a given config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionPlan {
    /// Nothing to provision.
    Noop,
    /// Write each file to the boot partition root.
    CloudInit { files: Vec<ProvisionFile> },
    /// Write `script`, then append `cmdline_append` to `cmdline.txt` (idempotently).
    FirstRun {
        script: ProvisionFile,
        cmdline_append: String,
    },
}

/// Build the provisioning plan for `config`.
pub fn plan(config: &ProvisionConfig) -> ProvisionPlan {
    if config.is_empty() {
        return ProvisionPlan::Noop;
    }
    match config.init_format {
        InitFormat::CloudInit => ProvisionPlan::CloudInit {
            files: cloud_init_files(config),
        },
        InitFormat::FirstRun => ProvisionPlan::FirstRun {
            script: ProvisionFile {
                name: "firstrun.sh".into(),
                contents: firstrun_script(config),
                executable: true,
            },
            cmdline_append: FIRSTRUN_CMDLINE.to_string(),
        },
    }
}

fn cloud_init_files(config: &ProvisionConfig) -> Vec<ProvisionFile> {
    let mut files = Vec::new();

    if let Some(user_data) = cloud_init_user_data(config) {
        files.push(ProvisionFile::plain("user-data", user_data));
    }

    if let Some(wifi) = &config.wifi {
        files.push(ProvisionFile::plain("network-config", network_config_v2(wifi)));
    }

    // cloud-init's NoCloud datasource needs meta-data to exist.
    files.push(ProvisionFile::plain("meta-data", String::new()));
    files
}

fn cloud_init_user_data(config: &ProvisionConfig) -> Option<String> {
    let mut top = String::new();
    let mut write_files: Vec<String> = Vec::new();
    let mut runcmd: Vec<String> = Vec::new();
    let mut scrub = false;

    if let Some(host) = &config.hostname {
        top.push_str(&format!("hostname: {host}\npreserve_hostname: false\n"));
    }

    if let Some(access) = config.access.as_ref().filter(|a| a.is_effective()) {
        // Returns true when it wrote a plaintext secret (a password) that must
        // not linger on the card.
        scrub |= access_cloud_init(access, &mut top, &mut runcmd);
    }

    if let Some((control, key)) = config.tailscale.as_ref().and_then(provisioned_tailscale) {
        write_files.push(tailscale_write_files_entry(&control, &key));
        scrub = true;
    }

    if scrub {
        runcmd.push(SCRUB_USERDATA_ITEM.to_string());
    }

    if top.is_empty() && write_files.is_empty() && runcmd.is_empty() {
        return None;
    }

    let mut body = top;
    if !write_files.is_empty() {
        body.push_str("write_files:\n");
        for entry in &write_files {
            body.push_str(entry);
        }
    }
    if !runcmd.is_empty() {
        body.push_str("runcmd:\n");
        for entry in &runcmd {
            body.push_str(entry);
        }
    }
    Some(format!("#cloud-config\n{body}"))
}

/// Emits the SSH/credential cloud-config into `top` (and a disable-ssh item into
/// `runcmd`). Returns true if a plaintext secret (password) was written, so the
/// caller can scrub `user-data` after first boot. The public key is not a secret.
fn access_cloud_init(access: &AccessConfig, top: &mut String, runcmd: &mut Vec<String>) -> bool {
    match access.ssh {
        SshMode::KeyOnly => {
            if let Some(key) = sanitized(&access.authorized_key) {
                top.push_str("ssh_pwauth: false\n");
                top.push_str(&format!("ssh_authorized_keys:\n  - {}\n", json_string(&key)));
            }
            false
        }
        SshMode::Password => {
            if let Some(pw) = sanitized(&access.password) {
                top.push_str("ssh_pwauth: true\n");
                top.push_str(&format!(
                    "chpasswd:\n  expire: false\n  users:\n    - {{name: pi, password: {}, type: text}}\n",
                    json_string(&pw)
                ));
                return true;
            }
            false
        }
        SshMode::Disabled => {
            // Disable both the service and the socket: on socket-activated SSH,
            // stopping only ssh.service would leave ssh.socket listening. The
            // `; true` tolerates a system that has only one of them.
            runcmd.push(
                "  - [ sh, -c, \"systemctl disable --now ssh.socket ssh.service 2>/dev/null; true\" ]\n"
                    .to_string(),
            );
            false
        }
    }
}

/// One sanitized, non-blank line, or `None`. Strips control characters so a
/// pasted value can't break out of the surrounding YAML.
fn sanitized(v: &Option<String>) -> Option<String> {
    let s = sanitize_line(v.as_deref().unwrap_or_default());
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Removes the seed + cloud-init's cache of user-data after first boot, so a
/// plaintext secret (Tailscale key, device password) doesn't linger on the card
/// or root filesystem. Runs after cloud-init has already applied everything.
const SCRUB_USERDATA_ITEM: &str = "  - [ sh, -c, \"rm -f /boot/firmware/user-data /var/lib/cloud/instance/user-data.txt /var/lib/cloud/instances/*/user-data.txt 2>/dev/null; true\" ]\n";

fn tailscale_write_files_entry(control_server: &str, auth_key: &str) -> String {
    let json = tailscale_state_json(control_server);
    format!(
        "  - path: {TS_STATE_PATH}\n    permissions: '0644'\n    content: |\n      {json}\n  - path: {TS_KEY_PATH}\n    permissions: '0600'\n    content: |\n      {key}\n",
        key = auth_key,
    )
}

/// cloud-init v2 network-config for a single wlan0 network, using a derived
/// WPA-PSK rather than the plaintext passphrase.
fn network_config_v2(wifi: &WifiConfig) -> String {
    let key = psk::derive(&wifi.ssid, &wifi.password);
    format!(
        "version: 2\n\
         wifis:\n  \
         wlan0:\n    \
         dhcp4: true\n    \
         optional: true\n    \
         regulatory-domain: \"{country}\"\n    \
         access-points:\n      \
         \"{ssid}\":\n        \
         password: \"{key}\"\n",
        country = wifi.country,
        ssid = wifi.ssid,
    )
}

/// Legacy `firstrun.sh` that sets the hostname and writes a wpa_supplicant.conf
/// with the derived PSK, then self-cleans.
fn firstrun_script(config: &ProvisionConfig) -> String {
    let mut s = String::from("#!/bin/bash\n\nset +e\n\n");

    if let Some(host) = &config.hostname {
        s.push_str(&format!(
            "CURRENT_HOSTNAME=$(cat /etc/hostname | tr -d \" \\t\\n\\r\")\n\
             if [ -f /usr/lib/raspberrypi-sys-mods/imager_custom ]; then\n\
             \x20  /usr/lib/raspberrypi-sys-mods/imager_custom set_hostname {host}\n\
             else\n\
             \x20  echo {host} >/etc/hostname\n\
             \x20  sed -i \"s/127.0.1.1.*$CURRENT_HOSTNAME/127.0.1.1\\t{host}/g\" /etc/hosts\n\
             fi\n\n"
        ));
    }

    if let Some(wifi) = &config.wifi {
        let key = psk::derive(&wifi.ssid, &wifi.password);
        s.push_str(&format!(
            "cat >/etc/wpa_supplicant/wpa_supplicant.conf <<'WPAEOF'\n\
             ctrl_interface=DIR=/var/run/wpa_supplicant GROUP=netdev\n\
             update_config=1\n\
             country={country}\n\
             network={{\n\
             \x20  ssid=\"{ssid}\"\n\
             \x20  psk={key}\n\
             }}\n\
             WPAEOF\n\
             rfkill unblock wifi\n\
             for f in /var/lib/systemd/rfkill/*:wlan ; do echo 0 >\"$f\" 2>/dev/null || true ; done\n\n",
            country = wifi.country,
            ssid = wifi.ssid,
        ));
    }

    if let Some((control, key)) = config.tailscale.as_ref().and_then(provisioned_tailscale) {
        let json = tailscale_state_json(&control);
        s.push_str(&format!(
            "mkdir -p /var/lib/aircastd\n\
             cat >{TS_STATE_PATH} <<'AIRCASTTSEOF'\n\
             {json}\n\
             AIRCASTTSEOF\n\
             cat >{TS_KEY_PATH} <<'AIRCASTKEYEOF'\n\
             {key}\n\
             AIRCASTKEYEOF\n\
             chmod 600 {TS_KEY_PATH}\n\n",
        ));
    }

    if let Some(access) = config.access.as_ref().filter(|a| a.is_effective()) {
        s.push_str(&firstrun_access(access));
    }

    s.push_str("rm -f /boot/firstrun.sh\n");
    s.push_str("sed -i 's| systemd.run=.*||g' /boot/cmdline.txt 2>/dev/null || true\n");
    s.push_str("exit 0\n");
    s
}

/// Shell for the legacy firstrun path that applies the same SSH/credential
/// posture as [`access_cloud_init`]. Values are single-quoted in the shell and
/// pre-sanitized of control characters so a pasted value can't break out.
fn firstrun_access(access: &AccessConfig) -> String {
    match access.ssh {
        SshMode::KeyOnly => {
            let Some(key) = sanitized(&access.authorized_key) else {
                return String::new();
            };
            format!(
                "install -d -m 700 -o pi -g pi /home/pi/.ssh\n\
                 printf '%s\\n' '{key}' >>/home/pi/.ssh/authorized_keys\n\
                 chmod 600 /home/pi/.ssh/authorized_keys\n\
                 chown pi:pi /home/pi/.ssh/authorized_keys\n\
                 sed -i 's/^#\\?PasswordAuthentication.*/PasswordAuthentication no/' /etc/ssh/sshd_config\n\
                 systemctl enable ssh\n\n",
                key = shell_single_quote(&key),
            )
        }
        SshMode::Password => {
            let Some(pw) = sanitized(&access.password) else {
                return String::new();
            };
            format!(
                "printf 'pi:%s\\n' '{pw}' | chpasswd\n\
                 sed -i 's/^#\\?PasswordAuthentication.*/PasswordAuthentication yes/' /etc/ssh/sshd_config\n\
                 systemctl enable ssh\n\n",
                pw = shell_single_quote(&pw),
            )
        }
        SshMode::Disabled => {
            "systemctl disable --now ssh.socket ssh.service 2>/dev/null || true\n\n".to_string()
        }
    }
}

/// Escapes a sanitized value for safe inclusion inside a shell single-quoted
/// string (`'...'`), turning each `'` into `'\''`.
fn shell_single_quote(s: &str) -> String {
    s.replace('\'', "'\\''")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(init: InitFormat, with_wifi: bool, host: Option<&str>) -> ProvisionConfig {
        ProvisionConfig {
            hostname: host.map(String::from),
            wifi: with_wifi.then(|| WifiConfig {
                ssid: "IEEE".into(),
                password: "password".into(),
                country: "US".into(),
            }),
            tailscale: None,
            access: None,
            init_format: init,
        }
    }

    fn ts_cfg(init: InitFormat, host: Option<&str>) -> ProvisionConfig {
        ProvisionConfig {
            hostname: host.map(String::from),
            wifi: None,
            tailscale: Some(TailscaleConfig {
                control_server: "https://headscale.example.com".into(),
                auth_key: "hskey-auth-abc123".into(),
            }),
            access: None,
            init_format: init,
        }
    }

    fn access_cfg(init: InitFormat, access: AccessConfig) -> ProvisionConfig {
        ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: None,
            access: Some(access),
            init_format: init,
        }
    }

    fn cloud_user_data(config: &ProvisionConfig) -> String {
        let ProvisionPlan::CloudInit { files } = plan(config) else {
            panic!("expected CloudInit plan");
        };
        files
            .into_iter()
            .find(|f| f.name == "user-data")
            .expect("user-data file")
            .contents
    }

    #[test]
    fn empty_config_is_noop() {
        let plan = plan(&cfg(InitFormat::CloudInit, false, None));
        assert_eq!(plan, ProvisionPlan::Noop);
    }

    #[test]
    fn cloud_init_writes_psk_not_plaintext() {
        let ProvisionPlan::CloudInit { files } =
            plan(&cfg(InitFormat::CloudInit, true, Some("aircast")))
        else {
            panic!("expected CloudInit plan");
        };
        let net = files.iter().find(|f| f.name == "network-config").unwrap();
        // The IEEE/password vector PSK, never the plaintext.
        assert!(net
            .contents
            .contains("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"));
        assert!(!net.contents.contains("password: \"password\""));
        assert!(files.iter().any(|f| f.name == "user-data" && f.contents.contains("hostname: aircast")));
        assert!(files.iter().any(|f| f.name == "meta-data"));
    }

    #[test]
    fn firstrun_writes_script_and_cmdline() {
        let ProvisionPlan::FirstRun { script, cmdline_append } =
            plan(&cfg(InitFormat::FirstRun, true, Some("aircast")))
        else {
            panic!("expected FirstRun plan");
        };
        assert_eq!(script.name, "firstrun.sh");
        assert!(script.executable);
        assert!(script.contents.contains("set_hostname aircast"));
        assert!(script
            .contents
            .contains("psk=f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"));
        assert!(cmdline_append.contains("systemd.run=/boot/firstrun.sh"));
    }

    #[test]
    fn tailscale_only_config_is_not_noop() {
        assert!(!ts_cfg(InitFormat::CloudInit, None).is_empty());
        assert!(!matches!(
            plan(&ts_cfg(InitFormat::CloudInit, None)),
            ProvisionPlan::Noop
        ));
    }

    #[test]
    fn cloud_init_provisions_tailscale_store() {
        let ProvisionPlan::CloudInit { files } = plan(&ts_cfg(InitFormat::CloudInit, None)) else {
            panic!("expected CloudInit plan");
        };
        let user_data = files.iter().find(|f| f.name == "user-data").unwrap();
        assert!(user_data.contents.contains("write_files:"));
        assert!(user_data
            .contents
            .contains("path: /var/lib/aircastd/tailscale.json"));
        assert!(user_data
            .contents
            .contains("{\"loginServer\":\"https://headscale.example.com\"}"));
        assert!(user_data
            .contents
            .contains("path: /var/lib/aircastd/tailscale.authkey"));
        assert!(user_data.contents.contains("permissions: '0600'"));
        assert!(user_data.contents.contains("hskey-auth-abc123"));
        assert!(user_data.contents.contains("runcmd:"));
        assert!(user_data
            .contents
            .contains("/var/lib/cloud/instance/user-data.txt"));
    }

    #[test]
    fn firstrun_provisions_tailscale_store() {
        let ProvisionPlan::FirstRun { script, .. } = plan(&ts_cfg(InitFormat::FirstRun, None)) else {
            panic!("expected FirstRun plan");
        };
        assert!(script
            .contents
            .contains("cat >/var/lib/aircastd/tailscale.json"));
        assert!(script
            .contents
            .contains("{\"loginServer\":\"https://headscale.example.com\"}"));
        assert!(script
            .contents
            .contains("cat >/var/lib/aircastd/tailscale.authkey"));
        assert!(script.contents.contains("hskey-auth-abc123"));
        assert!(script
            .contents
            .contains("chmod 600 /var/lib/aircastd/tailscale.authkey"));
    }

    #[test]
    fn json_string_escapes_quotes_and_backslashes() {
        assert_eq!(json_string("a\"b\\c"), "\"a\\\"b\\\\c\"");
    }

    fn access(ssh: SshMode, key: Option<&str>, pw: Option<&str>) -> AccessConfig {
        AccessConfig {
            ssh,
            authorized_key: key.map(String::from),
            password: pw.map(String::from),
        }
    }

    #[test]
    fn access_key_only_with_no_key_is_noop() {
        let config = access_cfg(InitFormat::CloudInit, access(SshMode::KeyOnly, None, None));
        assert!(config.is_empty());
        assert_eq!(plan(&config), ProvisionPlan::Noop);
    }

    #[test]
    fn access_password_with_no_password_is_noop() {
        let config = access_cfg(InitFormat::CloudInit, access(SshMode::Password, None, None));
        assert!(config.is_empty());
    }

    #[test]
    fn access_disabled_is_always_effective() {
        let config = access_cfg(InitFormat::CloudInit, access(SshMode::Disabled, None, None));
        assert!(!config.is_empty());
        let ud = cloud_user_data(&config);
        assert!(ud.contains("runcmd:"));
        assert!(ud.contains("systemctl disable --now ssh.socket ssh.service"));
    }

    #[test]
    fn access_key_only_disables_password_auth_and_adds_key() {
        let config = access_cfg(
            InitFormat::CloudInit,
            access(SshMode::KeyOnly, Some("ssh-ed25519 AAAAkey operator@base"), None),
        );
        let ud = cloud_user_data(&config);
        assert!(ud.contains("ssh_pwauth: false"));
        assert!(ud.contains("ssh_authorized_keys:"));
        assert!(ud.contains("\"ssh-ed25519 AAAAkey operator@base\""));
        // A public key is not a secret — user-data is not scrubbed for it.
        assert!(!ud.contains("rm -f /boot/firmware/user-data"));
    }

    #[test]
    fn access_password_sets_pi_password_and_scrubs_userdata() {
        let config = access_cfg(
            InitFormat::CloudInit,
            access(SshMode::Password, None, Some("s3cret-pass")),
        );
        let ud = cloud_user_data(&config);
        assert!(ud.contains("ssh_pwauth: true"));
        assert!(ud.contains("chpasswd:"));
        assert!(ud.contains("{name: pi, password: \"s3cret-pass\", type: text}"));
        // The plaintext password must be removed from the card after first boot.
        assert!(ud.contains("rm -f /boot/firmware/user-data"));
    }

    #[test]
    fn access_value_newlines_are_stripped() {
        let config = access_cfg(
            InitFormat::CloudInit,
            access(SshMode::KeyOnly, Some("ssh-ed25519 KEY\nssh_pwauth: true"), None),
        );
        let ud = cloud_user_data(&config);
        // The injected newline is stripped, so it can't become its own YAML key.
        assert!(ud.contains("\"ssh-ed25519 KEYssh_pwauth: true\""));
    }

    #[test]
    fn disabled_ssh_and_tailscale_share_one_runcmd() {
        let mut config = ts_cfg(InitFormat::CloudInit, None);
        config.access = Some(access(SshMode::Disabled, None, None));
        let ud = cloud_user_data(&config);
        // Exactly one runcmd: block, carrying both the disable and the scrub.
        assert_eq!(ud.matches("runcmd:").count(), 1);
        assert!(ud.contains("systemctl disable --now ssh.socket ssh.service"));
        assert!(ud.contains("rm -f /boot/firmware/user-data"));
    }

    #[test]
    fn firstrun_password_sets_pi_password() {
        let config = access_cfg(
            InitFormat::FirstRun,
            access(SshMode::Password, None, Some("s3cret")),
        );
        let ProvisionPlan::FirstRun { script, .. } = plan(&config) else {
            panic!("expected FirstRun plan");
        };
        assert!(script.contents.contains("printf 'pi:%s\\n' 's3cret' | chpasswd"));
        assert!(script.contents.contains("PasswordAuthentication yes"));
    }

    #[test]
    fn firstrun_key_only_escapes_single_quotes() {
        let config = access_cfg(
            InitFormat::FirstRun,
            access(SshMode::KeyOnly, Some("key-with-'quote"), None),
        );
        let ProvisionPlan::FirstRun { script, .. } = plan(&config) else {
            panic!("expected FirstRun plan");
        };
        assert!(script.contents.contains(r"key-with-'\''quote"));
        assert!(script.contents.contains("PasswordAuthentication no"));
    }

    #[test]
    fn empty_auth_key_is_not_provisioned() {
        let mut config = ts_cfg(InitFormat::CloudInit, None);
        config.tailscale.as_mut().unwrap().auth_key = "".into();
        let ProvisionPlan::CloudInit { files } = plan(&config) else {
            panic!("expected CloudInit plan");
        };
        assert!(files.iter().all(|f| !f.contents.contains("write_files")));
    }

    #[test]
    fn tailscale_values_are_stripped_to_a_single_line() {
        let mut config = ts_cfg(InitFormat::CloudInit, None);
        let ts = config.tailscale.as_mut().unwrap();
        ts.control_server = "https://h\nevil: true".into();
        ts.auth_key = "hskey\nmore".into();
        let ProvisionPlan::CloudInit { files } = plan(&config) else {
            panic!("expected CloudInit plan");
        };
        let user_data = &files.iter().find(|f| f.name == "user-data").unwrap().contents;
        // Newlines stripped → values concatenated, never a fresh YAML line.
        assert!(user_data.contains("https://hevil: true"));
        assert!(user_data.contains("hskeymore"));
        assert!(!user_data.contains("\nevil: true"));
    }
}
