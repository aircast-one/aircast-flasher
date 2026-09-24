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
        // manage_etc_hosts keeps 127.0.1.1 pointing at the name cloud-init just
        // set. Without it the image's own entry survives, every sudo prints
        // "unable to resolve host", and anything resolving its own hostname
        // waits for DNS to fail first.
        top.push_str(&format!(
            "hostname: {host}\npreserve_hostname: false\nmanage_etc_hosts: localhost\n"
        ));
    }

    if let Some(access) = config.access.as_ref().filter(|a| a.is_effective()) {
        // Returns true when it wrote a plaintext secret (a password) that must
        // not linger on the card.
        scrub |= access_cloud_init(access, &mut top, &mut write_files, &mut runcmd);
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

/// Emits the SSH/credential cloud-config: settings into `top`, the staged key
/// and its install script into `write_files`, and the item that runs it — or
/// disables ssh — into `runcmd`. Returns true if a plaintext secret (password)
/// was written, so the caller can scrub `user-data` after first boot. The
/// public key is not a secret.
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
                // Not top-level `ssh_authorized_keys`: cloud-init hands those
                // to its default user, and this image has none — the login user
                // is created by pi-gen, not by cloud-init. The key then reached
                // nobody while ssh_pwauth: false still landed, which is a card
                // with no way into it at all. Install it by name instead, and
                // let the script put passwords back if even that fails.
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

const KEY_STAGE_PATH: &str = "/root/.aircast-authorized-key";
const INSTALL_KEY_PATH: &str = "/root/aircast-install-key.sh";
const RECOVERY_MARKER: &str = "/var/log/aircast-key-install-failed";
/// sshd takes the first value it reads and `Include /etc/ssh/sshd_config.d/*.conf`
/// sits at the top of Debian's sshd_config, so a drop-in decides this setting
/// whatever the main file says. cloud-init writes `50-cloud-init.conf`; glob
/// order means anything that must beat it has to sort *before* it, not after.
const ACCESS_DROPIN: &str = "/etc/ssh/sshd_config.d/10-aircast-access.conf";

/// The first-boot step that puts the operator's key where sshd will read it and
/// then settles password auth to match what actually happened.
///
/// It resolves the account rather than assuming any part of it: uid 1000 is
/// what pi-gen's `FIRST_USER_NAME` creates, and the group and home come out of
/// that same passwd entry — a group is not always named after its user, and
/// assuming a name is what broke this in the first place.
///
/// One rule, whichever path emitted it: the key is in place, so passwords go
/// off; or it is not, so passwords stay on and a marker says why. Handing over
/// a card with neither a key nor a password is the one outcome worth avoiding.
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

/// Stages the key next to the script, so neither the YAML nor the shell has to
/// quote it: the key never appears in a command line.
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
            // The same body cloud-init runs: one place decides which account
            // the key belongs to and whether passwords may go off.
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
        assert!(ud.contains(&format!("  - path: {KEY_STAGE_PATH}")));
        assert!(ud.contains("      ssh-ed25519 AAAAkey operator@base\n"));
        assert!(ud.contains(&format!("  - [ sh, {INSTALL_KEY_PATH} ]")));
        // A public key is not a secret — user-data is not scrubbed for it.
        assert!(!ud.contains("rm -f /boot/firmware/user-data"));
    }

    #[test]
    fn key_only_never_leaves_the_key_to_cloud_inits_default_user() {
        // This image has no cloud-init default user, so a top-level
        // ssh_authorized_keys reaches nobody while ssh_pwauth: false still
        // applies — which is exactly how a card ends up with no way in.
        let config = access_cfg(
            InitFormat::CloudInit,
            access(SshMode::KeyOnly, Some("ssh-ed25519 AAAAkey operator@base"), None),
        );
        let ud = cloud_user_data(&config);
        assert!(!ud.contains("ssh_authorized_keys:"));
        assert!(ud.contains("  - [ sh, /root/aircast-install-key.sh ]"));
    }

    #[test]
    fn install_key_script_resolves_the_user_instead_of_assuming_one() {
        // Assuming the name is what broke this the first time: the account is
        // pi-gen's FIRST_USER_NAME, which the flasher does not choose.
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
        // Handing over a card with neither a key nor a password is the one
        // outcome worth avoiding, so the failure branch turns passwords on.
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
        // A user-data that does not parse is ignored in silence at boot, which
        // is the whole failure mode here: assert the shape, not a substring.
        let ud = cloud_user_data(&access_cfg(
            InitFormat::CloudInit,
            access(SshMode::KeyOnly, Some("ssh-ed25519 AAAAkey operator@base"), None),
        ));
        let doc: serde_norway::Value = serde_norway::from_str(&ud).expect("valid YAML");
        assert_eq!(doc["ssh_pwauth"].as_bool(), Some(false));
        assert!(doc.get("ssh_authorized_keys").is_none());

        let files = doc["write_files"].as_sequence().expect("write_files is a list");
        let staged = files
            .iter()
            .find(|f| f["path"].as_str() == Some(KEY_STAGE_PATH))
            .expect("the key is staged");
        assert_eq!(staged["content"].as_str(), Some("ssh-ed25519 AAAAkey operator@base\n"));
        assert_eq!(staged["permissions"].as_str(), Some("0600"));
        let script = files
            .iter()
            .find(|f| f["path"].as_str() == Some(INSTALL_KEY_PATH))
            .expect("the script is written");
        assert!(script["content"].as_str().unwrap().starts_with("#!/bin/sh\n"));

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
        // sshd takes the first value it reads and cloud-init writes
        // 50-cloud-init.conf, so editing the main config or sorting after it
        // would change nothing at all.
        assert!(ACCESS_DROPIN.starts_with("/etc/ssh/sshd_config.d/"));
        let name = ACCESS_DROPIN.rsplit('/').next().unwrap();
        assert!(name < "50-cloud-init.conf", "{name} must sort before it");
        assert!(install_key_script().contains(ACCESS_DROPIN));
    }

    #[test]
    fn install_key_script_checks_the_key_itself_not_just_a_non_empty_file() {
        // An image that ships its own authorized_keys would otherwise pass a
        // -s test with the operator's key nowhere in it.
        let script = install_key_script();
        assert!(script.contains("grep -qxF \"$(cat \"$key\")\" \"$home/.ssh/authorized_keys\""));
    }

    #[test]
    fn both_init_formats_run_the_same_install_body() {
        // The duplicate that hardcoded pi in one path and resolved it in the
        // other is exactly how these two drifted apart before.
        let body = install_key_body();
        let cloud = cloud_user_data(&access_cfg(
            InitFormat::CloudInit,
            access(SshMode::KeyOnly, Some("ssh-ed25519 AAAAkey operator@base"), None),
        ));
        let ProvisionPlan::FirstRun { script, .. } = plan(&access_cfg(
            InitFormat::FirstRun,
            access(SshMode::KeyOnly, Some("ssh-ed25519 AAAAkey operator@base"), None),
        )) else {
            panic!("expected FirstRun plan");
        };
        // Every line, not a token one: the two paths drifted apart last time in
        // the lines this test would have skipped.
        for line in body.lines().filter(|l| !l.trim().is_empty()) {
            assert!(script.contents.contains(line), "firstrun is missing: {line}");
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
        // The injected newline is stripped, so it can't become its own YAML key:
        // it stays one indented line inside the staged key's block scalar.
        assert!(ud.contains("      ssh-ed25519 KEYssh_pwauth: true\n"));
        assert!(!ud.lines().any(|l| l == "ssh_pwauth: true"));
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
        // The shared body decides the setting from whether the key landed, so
        // firstrun no longer hardcodes it — nor the account it belongs to.
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
