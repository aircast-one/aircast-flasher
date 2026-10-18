use std::fs::{File, OpenOptions};
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};

use flasher_core::{
    flash, BlockDevice, EncryptionConfig, FlashParams, InitFormat, ProvisionConfig, TangServer,
    WifiConfig,
};

struct Card(File);

impl Read for Card {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for Card {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl Seek for Card {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        self.0.seek(pos)
    }
}

impl BlockDevice for Card {
    fn sync(&mut self) -> io::Result<()> {
        self.0.sync_all()
    }
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.windows(2).find(|w| w[0] == name).map(|w| w[1].clone())
}

fn required(args: &[String], name: &str) -> Result<String, String> {
    arg(args, name).ok_or_else(|| format!("{name} is required"))
}

fn encryption(args: &[String]) -> Result<Option<EncryptionConfig>, String> {
    let Some(url) = arg(args, "--tang-url") else {
        return Ok(None);
    };
    let adv_path = required(args, "--tang-adv")?;
    let adv = std::fs::read_to_string(&adv_path).map_err(|e| format!("{adv_path}: {e}"))?;
    Ok(Some(EncryptionConfig {
        tang_servers: vec![TangServer { url, adv }],
        threshold: 1,
        recovery_key: required(args, "--recovery-key")?,
    }))
}

fn wifi(args: &[String]) -> Option<WifiConfig> {
    let ssid = arg(args, "--wifi-ssid")?;
    Some(WifiConfig {
        ssid,
        password: arg(args, "--wifi-password").unwrap_or_default(),
        country: arg(args, "--wifi-country").unwrap_or_else(|| "US".into()),
    })
}

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    let image_path = required(&args, "--image")?;
    let card_path = required(&args, "--card")?;
    let card_bytes: u64 = required(&args, "--card-bytes")?
        .parse()
        .map_err(|e| format!("--card-bytes: {e}"))?;

    let provision = ProvisionConfig {
        hostname: None,
        wifi: wifi(&args),
        tailscale: None,
        access: None,
        encryption: encryption(&args)?,
        init_format: InitFormat::CloudInit,
    };

    let image = File::open(&image_path).map_err(|e| format!("{image_path}: {e}"))?;
    let image_len = image
        .metadata()
        .map_err(|e| format!("{image_path}: {e}"))?
        .len();
    let card = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(&card_path)
        .map_err(|e| format!("{card_path}: {e}"))?;
    card.set_len(card_bytes)
        .map_err(|e| format!("size {card_path}: {e}"))?;

    flash(
        &mut Card(card),
        &mut BufReader::new(image),
        &FlashParams {
            image_len,
            verify: true,
        },
        &provision,
        &mut |_, _, _| {},
    )
}
