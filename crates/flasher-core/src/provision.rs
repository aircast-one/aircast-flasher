use crate::{psk, AccessConfig, InitFormat, ProvisionConfig, SshMode, TailscaleConfig, WifiConfig};

const ENCRYPT_MARKER_NAME: &str = "aircast-encrypt";
const FIGHT_MARKER_NAME: &str = "aircast-fight-mode";

const TS_STATE_PATH: &str = "/var/lib/aircastd/tailscale.json";
const TS_KEY_PATH: &str = "/var/lib/aircastd/tailscale.authkey";

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvisionFile {
    pub name: String,
    pub contents: String,
    pub executable: bool,
}

impl ProvisionFile {
    fn plain(name: &str, contents: String) -> Self {
        Self {
            name: name.into(),
            contents,
            executable: false,
        }
    }
}

pub const FIRSTRUN_CMDLINE: &str =
    " systemd.run=/boot/firstrun.sh systemd.run_success_action=reboot systemd.unit=kernel-command-line.target";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProvisionPlan {
    Noop,
    CloudInit {
        files: Vec<ProvisionFile>,
    },
    FirstRun {
        script: ProvisionFile,
        cmdline_append: String,
    },
}

pub fn boot_markers(config: &ProvisionConfig) -> Vec<ProvisionFile> {
    encryption_files(config)
        .into_iter()
        .chain(
            config
                .fight_mode
                .then(|| ProvisionFile::plain(FIGHT_MARKER_NAME, String::new())),
        )
        .collect()
}

pub fn encryption_files(config: &ProvisionConfig) -> Vec<ProvisionFile> {
    let Some(enc) = config.encryption.as_ref().filter(|e| e.is_complete()) else {
        return Vec::new();
    };
    let Some(clevis_cfg) = clevis_sss_config(enc) else {
        return Vec::new();
    };
    let mut marker = format!(
        "# aircast disk-encryption enrolment seed — consumed and wiped by\n\
         # aircast-firstboot-data.sh on first boot. Holds a clear recovery key;\n\
         # do not keep a copy of this file.\n\
         CLEVIS_CFG='{cfg}'\n\
         RECOVERY_KEY='{key}'\n",
        cfg = shell_single_quote(&sanitize_line(&clevis_cfg)),
        key = shell_single_quote(&sanitize_line(&enc.recovery_key)),
    );
    if let Some(wifi) = &config.wifi {
        marker.push_str(&format!(
            "WIFI_SSID_HEX='{ssid}'\n\
             WIFI_PSK='{psk}'\n\
             WIFI_COUNTRY='{country}'\n",
            ssid = hex::encode(wifi.ssid.as_bytes()),
            psk = shell_single_quote(&psk::derive(&wifi.ssid, &wifi.password)),
            country = shell_single_quote(&sanitize_line(&wifi.country)),
        ));
    }
    vec![ProvisionFile::plain(ENCRYPT_MARKER_NAME, marker)]
}

fn clevis_sss_config(enc: &crate::EncryptionConfig) -> Option<String> {
    let pins = enc
        .tang_servers
        .iter()
        .map(|s| {
            let adv: serde_json::Value = serde_json::from_str(&s.adv).ok()?;
            Some(serde_json::json!({ "url": s.url.trim(), "adv": adv }))
        })
        .collect::<Option<Vec<_>>>()?;
    let cfg = serde_json::json!({
        "t": enc.threshold,
        "pins": { "tang": pins },
    });
    Some(cfg.to_string())
}

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
        files.push(ProvisionFile::plain(
            "network-config",
            network_config_v2(wifi),
        ));
    }

    files.push(ProvisionFile::plain("meta-data", String::new()));
    files
}

fn cloud_init_user_data(config: &ProvisionConfig) -> Option<String> {
    let mut top = String::new();
    let mut write_files: Vec<String> = Vec::new();
    let mut runcmd: Vec<String> = Vec::new();
    let mut scrub = config.wifi.is_some();

    if let Some(host) = &config.hostname {
        top.push_str(&format!(
            "hostname: {host}\npreserve_hostname: false\nmanage_etc_hosts: localhost\n"
        ));
    }

    if let Some(access) = config.access.as_ref().filter(|a| a.is_effective()) {
        scrub |= access_cloud_init(access, &mut top, &mut write_files, &mut runcmd);
    }

    if let Some((control, key)) = config.tailscale.as_ref().and_then(provisioned_tailscale) {
        write_files.push(tailscale_write_files_entry(&control, &key));
        scrub = true;
    }

    if scrub {
        runcmd.push(SCRUB_SEED_ITEM.to_string());
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

fn access_cloud_init(
    access: &AccessConfig,
    top: &mut String,
    write_files: &mut Vec<String>,
    runcmd: &mut Vec<String>,
) -> bool {
    match access.ssh {
        SshMode::KeyOnly => {
            if let Some(key) = sanitized(&access.authorized_key) {
                top.push_str("ssh_pwauth: false\n");
                write_files.push(install_key_write_files(&key));
                runcmd.push(install_key_runcmd());
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
            runcmd.push(
                "  - [ sh, -c, \"systemctl disable --now ssh.socket ssh.service 2>/dev/null; true\" ]\n"
                    .to_string(),
            );
            false
        }
    }
}

fn sanitized(v: &Option<String>) -> Option<String> {
    let s = sanitize_line(v.as_deref().unwrap_or_default());
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

const KEY_STAGE_PATH: &str = "/root/.aircast-authorized-key";
const INSTALL_KEY_PATH: &str = "/root/aircast-install-key.sh";
const RECOVERY_MARKER: &str = "/var/log/aircast-key-install-failed";
const ACCESS_DROPIN: &str = "/etc/ssh/sshd_config.d/10-aircast-access.conf";

fn install_key_body() -> String {
    format!(
        "key={KEY_STAGE_PATH}\n\
         user=$(getent passwd 1000 | cut -d: -f1)\n\
         [ -n \"$user\" ] || user=pi\n\
         group=$(getent passwd \"$user\" | cut -d: -f4)\n\
         [ -n \"$group\" ] || group=\"$user\"\n\
         home=$(getent passwd \"$user\" | cut -d: -f6)\n\
         [ -n \"$home\" ] || home=/home/$user\n\
         landed=no\n\
         if [ -s \"$key\" ]; then\n\
         \x20 install -d -m 700 -o \"$user\" -g \"$group\" \"$home/.ssh\" \\\n\
         \x20   && cat \"$key\" >>\"$home/.ssh/authorized_keys\" \\\n\
         \x20   && chmod 600 \"$home/.ssh/authorized_keys\" \\\n\
         \x20   && chown \"$user\":\"$group\" \"$home/.ssh/authorized_keys\"\n\
         \x20 grep -qxF \"$(cat \"$key\")\" \"$home/.ssh/authorized_keys\" 2>/dev/null && landed=yes\n\
         fi\n\
         rm -f \"$key\"\n\
         mkdir -p /etc/ssh/sshd_config.d\n\
         if [ \"$landed\" = yes ]; then\n\
         \x20 want=no\n\
         \x20 rm -f {RECOVERY_MARKER}\n\
         else\n\
         \x20 want=yes\n\
         \x20 echo \"aircast: could not install the operator key for $user; passwords left on so the device stays reachable\" >{RECOVERY_MARKER}\n\
         fi\n\
         printf 'PasswordAuthentication %s\\n' \"$want\" >{ACCESS_DROPIN}\n\
         sed -i \"s/^#\\?PasswordAuthentication.*/PasswordAuthentication $want/\" /etc/ssh/sshd_config 2>/dev/null || true\n\
         systemctl try-restart ssh 2>/dev/null || systemctl try-restart sshd 2>/dev/null || true\n"
    )
}

fn install_key_script() -> String {
    format!("#!/bin/sh\n{}", install_key_body())
}

fn install_key_write_files(key: &str) -> String {
    let script = install_key_script()
        .lines()
        .map(|l| format!("      {l}\n"))
        .collect::<String>();
    format!(
        "  - path: {KEY_STAGE_PATH}\n    permissions: '0600'\n    content: |\n      {key}\n\
         \x20 - path: {INSTALL_KEY_PATH}\n    permissions: '0700'\n    content: |\n{script}"
    )
}

fn install_key_runcmd() -> String {
    format!("  - [ sh, {INSTALL_KEY_PATH} ]\n")
}

const SCRUB_SEED_ITEM: &str = "  - [ sh, -c, \"rm -f /boot/firmware/user-data /boot/firmware/network-config /var/lib/cloud/instance/user-data.txt /var/lib/cloud/instances/*/user-data.txt 2>/dev/null; true\" ]\n";

fn tailscale_write_files_entry(control_server: &str, auth_key: &str) -> String {
    let json = tailscale_state_json(control_server);
    format!(
        "  - path: {TS_STATE_PATH}\n    permissions: '0644'\n    content: |\n      {json}\n  - path: {TS_KEY_PATH}\n    permissions: '0600'\n    content: |\n      {key}\n",
        key = auth_key,
    )
}

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

fn firstrun_access(access: &AccessConfig) -> String {
    match access.ssh {
        SshMode::KeyOnly => {
            let Some(key) = sanitized(&access.authorized_key) else {
                return String::new();
            };
            format!(
                "printf '%s\\n' '{key}' >{KEY_STAGE_PATH}\n\
                 chmod 600 {KEY_STAGE_PATH}\n\
                 {body}\
                 systemctl enable ssh\n\n",
                key = shell_single_quote(&key),
                body = install_key_body(),
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
            encryption: None,
            fight_mode: false,
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
            encryption: None,
            fight_mode: false,
            init_format: init,
        }
    }

    fn access_cfg(init: InitFormat, access: AccessConfig) -> ProvisionConfig {
        ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: None,
            access: Some(access),
            encryption: None,
            fight_mode: false,
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
        assert!(net
            .contents
            .contains("f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e"));
        assert!(!net.contents.contains("password: \"password\""));
        assert!(files
            .iter()
            .any(|f| f.name == "user-data" && f.contents.contains("hostname: aircast")));
        assert!(files.iter().any(|f| f.name == "meta-data"));
    }

    #[test]
    fn cloud_init_renames_the_host_in_etc_hosts_too() {
        let ProvisionPlan::CloudInit { files } =
            plan(&cfg(InitFormat::CloudInit, true, Some("aircast")))
        else {
            panic!("expected CloudInit plan");
        };
        let user_data = files.iter().find(|f| f.name == "user-data").unwrap();
        assert!(user_data.contents.contains("manage_etc_hosts: localhost"));
    }

    #[test]
    fn firstrun_writes_script_and_cmdline() {
        let ProvisionPlan::FirstRun {
            script,
            cmdline_append,
        } = plan(&cfg(InitFormat::FirstRun, true, Some("aircast")))
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
        let ProvisionPlan::FirstRun { script, .. } = plan(&ts_cfg(InitFormat::FirstRun, None))
        else {
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
            access(
                SshMode::KeyOnly,
                Some("ssh-ed25519 AAAAkey operator@base"),
                None,
            ),
        );
        let ud = cloud_user_data(&config);
        assert!(ud.contains("ssh_pwauth: false"));
        assert!(ud.contains(&format!("  - path: {KEY_STAGE_PATH}")));
        assert!(ud.contains("      ssh-ed25519 AAAAkey operator@base\n"));
        assert!(ud.contains(&format!("  - [ sh, {INSTALL_KEY_PATH} ]")));
        assert!(!ud.contains("rm -f /boot/firmware/user-data"));
    }

    #[test]
    fn key_only_never_leaves_the_key_to_cloud_inits_default_user() {
        let config = access_cfg(
            InitFormat::CloudInit,
            access(
                SshMode::KeyOnly,
                Some("ssh-ed25519 AAAAkey operator@base"),
                None,
            ),
        );
        let ud = cloud_user_data(&config);
        assert!(!ud.contains("ssh_authorized_keys:"));
        assert!(ud.contains("  - [ sh, /root/aircast-install-key.sh ]"));
    }

    #[test]
    fn install_key_script_resolves_the_user_instead_of_assuming_one() {
        let script = install_key_script();
        assert!(script.contains("user=$(getent passwd 1000 | cut -d: -f1)"));
        assert!(script.contains("home=$(getent passwd \"$user\" | cut -d: -f6)"));
        assert!(script.contains("group=$(getent passwd \"$user\" | cut -d: -f4)"));
        assert!(script.contains("install -d -m 700 -o \"$user\" -g \"$group\" \"$home/.ssh\""));
        assert!(
            !script.contains("-g \"$user\""),
            "a group is not always named after its user"
        );
    }

    #[test]
    fn install_key_script_puts_passwords_back_when_the_key_does_not_land() {
        let script = install_key_script();
        assert!(
            !script.contains("|| exit 0"),
            "a missing staged key is a failed install, not a reason to leave passwords off"
        );
        let (landed, failed) = script
            .rsplit_once("else")
            .expect("the script branches on whether the key landed");
        assert!(landed.contains("want=no"));
        assert!(failed.contains("want=yes"));
        assert!(failed.contains(RECOVERY_MARKER));
    }

    #[test]
    fn key_only_user_data_is_valid_cloud_config() {
        let ud = cloud_user_data(&access_cfg(
            InitFormat::CloudInit,
            access(
                SshMode::KeyOnly,
                Some("ssh-ed25519 AAAAkey operator@base"),
                None,
            ),
        ));
        let doc: serde_norway::Value = serde_norway::from_str(&ud).expect("valid YAML");
        assert_eq!(doc["ssh_pwauth"].as_bool(), Some(false));
        assert!(doc.get("ssh_authorized_keys").is_none());

        let files = doc["write_files"]
            .as_sequence()
            .expect("write_files is a list");
        let staged = files
            .iter()
            .find(|f| f["path"].as_str() == Some(KEY_STAGE_PATH))
            .expect("the key is staged");
        assert_eq!(
            staged["content"].as_str(),
            Some("ssh-ed25519 AAAAkey operator@base\n")
        );
        assert_eq!(staged["permissions"].as_str(), Some("0600"));
        let script = files
            .iter()
            .find(|f| f["path"].as_str() == Some(INSTALL_KEY_PATH))
            .expect("the script is written");
        assert!(script["content"]
            .as_str()
            .unwrap()
            .starts_with("#!/bin/sh\n"));

        let run = doc["runcmd"].as_sequence().expect("runcmd is a list");
        let argv: Vec<&str> = run[0]
            .as_sequence()
            .expect("runcmd item is the argv list form, not a shell string")
            .iter()
            .map(|v| v.as_str().expect("argv entries are strings"))
            .collect();
        assert_eq!(argv, vec!["sh", INSTALL_KEY_PATH]);
    }

    #[test]
    fn recovery_beats_cloud_inits_own_sshd_drop_in() {
        assert!(ACCESS_DROPIN.starts_with("/etc/ssh/sshd_config.d/"));
        let name = ACCESS_DROPIN.rsplit('/').next().unwrap();
        assert!(name < "50-cloud-init.conf", "{name} must sort before it");
        assert!(install_key_script().contains(ACCESS_DROPIN));
    }

    #[test]
    fn install_key_script_checks_the_key_itself_not_just_a_non_empty_file() {
        let script = install_key_script();
        assert!(script.contains("grep -qxF \"$(cat \"$key\")\" \"$home/.ssh/authorized_keys\""));
    }

    #[test]
    fn both_init_formats_run_the_same_install_body() {
        let body = install_key_body();
        let cloud = cloud_user_data(&access_cfg(
            InitFormat::CloudInit,
            access(
                SshMode::KeyOnly,
                Some("ssh-ed25519 AAAAkey operator@base"),
                None,
            ),
        ));
        let ProvisionPlan::FirstRun { script, .. } = plan(&access_cfg(
            InitFormat::FirstRun,
            access(
                SshMode::KeyOnly,
                Some("ssh-ed25519 AAAAkey operator@base"),
                None,
            ),
        )) else {
            panic!("expected FirstRun plan");
        };
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            assert!(
                script.contents.contains(line),
                "firstrun is missing: {line}"
            );
            assert!(cloud.contains(line.trim()), "cloud-init is missing: {line}");
        }
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
        assert!(ud.contains("rm -f /boot/firmware/user-data"));
    }

    #[test]
    fn access_value_newlines_are_stripped() {
        let config = access_cfg(
            InitFormat::CloudInit,
            access(
                SshMode::KeyOnly,
                Some("ssh-ed25519 KEY\nssh_pwauth: true"),
                None,
            ),
        );
        let ud = cloud_user_data(&config);
        assert!(ud.contains("      ssh-ed25519 KEYssh_pwauth: true\n"));
        assert!(!ud.lines().any(|l| l == "ssh_pwauth: true"));
    }

    #[test]
    fn disabled_ssh_and_tailscale_share_one_runcmd() {
        let mut config = ts_cfg(InitFormat::CloudInit, None);
        config.access = Some(access(SshMode::Disabled, None, None));
        let ud = cloud_user_data(&config);
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
        assert!(script
            .contents
            .contains("printf 'pi:%s\\n' 's3cret' | chpasswd"));
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
        assert!(script.contents.contains("PasswordAuthentication %s"));
        assert!(!script.contents.contains("-o pi -g pi"));
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

    fn tang(url: &str) -> crate::TangServer {
        crate::TangServer {
            url: url.into(),
            adv: format!("{{\"payload\":\"{url}\",\"signatures\":[]}}"),
        }
    }

    fn enc_cfg() -> ProvisionConfig {
        let mut c = cfg(InitFormat::CloudInit, false, None);
        c.encryption = Some(crate::EncryptionConfig {
            tang_servers: vec![tang("http://10.0.0.5")],
            threshold: 1,
            recovery_key: "correct-horse-battery".into(),
        });
        c
    }

    #[test]
    fn encryption_marker_carries_an_sss_clevis_policy() {
        let files = encryption_files(&enc_cfg());
        let marker = &files
            .iter()
            .find(|f| f.name == "aircast-encrypt")
            .unwrap()
            .contents;
        assert!(marker.contains("RECOVERY_KEY='correct-horse-battery'"));
        assert_eq!(files.len(), 1);
        let cfg_line = marker
            .lines()
            .find(|l| l.starts_with("CLEVIS_CFG="))
            .unwrap();
        let json = cfg_line
            .trim_start_matches("CLEVIS_CFG=")
            .trim_matches('\'');
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["t"], 1);
        assert_eq!(v["pins"]["tang"][0]["url"], "http://10.0.0.5");
        assert_eq!(v["pins"]["tang"][0]["adv"]["payload"], "http://10.0.0.5");
    }

    #[test]
    fn encryption_policy_binds_multiple_tang_with_a_threshold() {
        let mut c = enc_cfg();
        let e = c.encryption.as_mut().unwrap();
        e.tang_servers = vec![tang("http://a"), tang("http://b"), tang("http://c")];
        e.threshold = 2;
        let marker = &encryption_files(&c)
            .into_iter()
            .find(|f| f.name == "aircast-encrypt")
            .unwrap()
            .contents;
        let json = marker
            .lines()
            .find(|l| l.starts_with("CLEVIS_CFG="))
            .unwrap()
            .trim_start_matches("CLEVIS_CFG=")
            .trim_matches('\'');
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["t"], 2);
        assert_eq!(v["pins"]["tang"].as_array().unwrap().len(), 3);
    }

    #[test]
    fn a_threshold_above_the_server_count_is_not_complete() {
        let mut c = enc_cfg();
        c.encryption.as_mut().unwrap().threshold = 2;
        assert!(encryption_files(&c).is_empty());
    }

    #[test]
    fn encryption_marker_carries_wifi_for_initramfs_unlock() {
        let mut c = enc_cfg();
        c.wifi = Some(WifiConfig {
            ssid: "field-net".into(),
            password: "password".into(),
            country: "UA".into(),
        });
        let marker = &encryption_files(&c)
            .into_iter()
            .find(|f| f.name == "aircast-encrypt")
            .unwrap()
            .contents;
        assert!(marker.contains("WIFI_SSID_HEX='6669656c642d6e6574'"));
        assert!(marker.contains("WIFI_COUNTRY='UA'"));
        assert!(marker.contains("WIFI_PSK='"));
        assert!(!marker.contains("password"));
    }

    #[test]
    fn encryption_marker_omits_wifi_when_wired() {
        let marker = &encryption_files(&enc_cfg())
            .into_iter()
            .find(|f| f.name == "aircast-encrypt")
            .unwrap()
            .contents;
        assert!(!marker.contains("WIFI_SSID"));
    }

    #[test]
    fn encryption_absent_or_incomplete_writes_nothing() {
        assert!(encryption_files(&cfg(InitFormat::CloudInit, false, None)).is_empty());
        let mut c = enc_cfg();
        c.encryption.as_mut().unwrap().tang_servers[0].url = "   ".into();
        assert!(encryption_files(&c).is_empty());
    }

    #[test]
    fn encryption_marker_values_cannot_break_out_of_the_shell_quote() {
        let mut c = enc_cfg();
        c.encryption.as_mut().unwrap().recovery_key = "a'; rm -rf /\n#".into();
        let files = encryption_files(&c);
        let marker = &files
            .iter()
            .find(|f| f.name == "aircast-encrypt")
            .unwrap()
            .contents;
        assert!(marker.contains(r"RECOVERY_KEY='a'\''; rm -rf /#'"));
        assert!(!marker.contains("rm -rf /\n"));
    }

    #[test]
    fn fight_mode_flags_the_boot_partition_for_aircastd() {
        let mut c = cfg(InitFormat::CloudInit, false, None);
        assert!(boot_markers(&c).is_empty());

        c.fight_mode = true;
        let markers = boot_markers(&c);
        assert_eq!(markers.len(), 1);
        assert_eq!(markers[0].name, "aircast-fight-mode");
        assert!(markers[0].contents.is_empty());
    }

    #[test]
    fn fight_mode_keeps_the_encryption_marker() {
        let mut c = enc_cfg();
        c.fight_mode = true;
        let names: Vec<_> = boot_markers(&c).into_iter().map(|f| f.name).collect();
        assert_eq!(names, ["aircast-encrypt", "aircast-fight-mode"]);
    }

    #[test]
    fn the_wifi_key_does_not_outlive_first_boot_on_the_boot_partition() {
        let ud = cloud_user_data(&cfg(InitFormat::CloudInit, true, None));
        assert!(ud.contains("rm -f /boot/firmware/user-data /boot/firmware/network-config"));
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
        let user_data = &files
            .iter()
            .find(|f| f.name == "user-data")
            .unwrap()
            .contents;
        assert!(user_data.contains("https://hevil: true"));
        assert!(user_data.contains("hskeymore"));
        assert!(!user_data.contains("\nevil: true"));
    }
}
