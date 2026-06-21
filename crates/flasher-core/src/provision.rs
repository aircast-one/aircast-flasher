//! Turn a [`ProvisionConfig`] into the concrete first-boot files to write onto
//! the FAT boot partition. Pure: no I/O. The Tauri app mounts the partition and
//! applies the returned [`ProvisionPlan`].

use crate::{psk, InitFormat, ProvisionConfig, WifiConfig};

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

    if let Some(host) = &config.hostname {
        let user_data = format!("#cloud-config\nhostname: {host}\npreserve_hostname: false\n");
        files.push(ProvisionFile::plain("user-data", user_data));
    }

    if let Some(wifi) = &config.wifi {
        files.push(ProvisionFile::plain("network-config", network_config_v2(wifi)));
    }

    // cloud-init's NoCloud datasource needs meta-data to exist.
    files.push(ProvisionFile::plain("meta-data", String::new()));
    files
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

    s.push_str("rm -f /boot/firstrun.sh\n");
    s.push_str("sed -i 's| systemd.run=.*||g' /boot/cmdline.txt 2>/dev/null || true\n");
    s.push_str("exit 0\n");
    s
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
            init_format: init,
        }
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
}
