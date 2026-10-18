use std::io::{Cursor, Read, Seek, SeekFrom, Write};
use std::time::{Duration, Instant};

use mbrman::MBR;
use sha2::{Digest, Sha256};

use crate::provision::{ProvisionFile, ProvisionPlan};
use crate::ProvisionConfig;

pub const BLOCK: usize = 1024 * 1024;

const SECTOR: u64 = 512;

const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);

pub trait BlockDevice: Read + Write + Seek {
    fn sync(&mut self) -> std::io::Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlashStage {
    Write,
    Verify,
    Customize,
}

pub struct FlashParams {
    pub image_len: u64,
    pub verify: bool,
}

pub fn flash(
    device: &mut dyn BlockDevice,
    image: &mut dyn Read,
    params: &FlashParams,
    provision: &ProvisionConfig,
    progress: &mut dyn FnMut(FlashStage, u64, u64),
) -> Result<(), String> {
    let image_len = params.image_len;

    let mut first_block = vec![0u8; BLOCK];
    let first_len = read_full(image, &mut first_block).map_err(|e| ioerr("read image", e))?;
    first_block.truncate(first_len);

    let mut write_hash = Sha256::new();
    write_hash.update(&first_block);

    zero_tail(device)?;

    device
        .seek(SeekFrom::Start(BLOCK as u64))
        .map_err(|e| ioerr("seek past block 0", e))?;

    let mut written = first_len as u64;
    progress(FlashStage::Write, written, image_len);
    let mut last_tick = Instant::now();

    let mut buf = vec![0u8; BLOCK];
    loop {
        let n = read_full(image, &mut buf).map_err(|e| ioerr("read image", e))?;
        if n == 0 {
            break;
        }
        write_hash.update(&buf[..n]);
        device
            .write_all(&buf[..n])
            .map_err(|e| ioerr("write to device", e))?;
        written += n as u64;

        if last_tick.elapsed() >= PROGRESS_INTERVAL {
            progress(FlashStage::Write, written, image_len);
            last_tick = Instant::now();
        }
        if n < BLOCK {
            break;
        }
    }
    progress(FlashStage::Write, written, image_len);

    device.sync().map_err(|e| ioerr("sync after write", e))?;

    let write_digest = write_hash.finalize();

    if params.verify {
        let mut verify_hash = Sha256::new();
        verify_hash.update(&first_block);

        device
            .seek(SeekFrom::Start(BLOCK as u64))
            .map_err(|e| ioerr("seek for verify", e))?;

        let mut checked = first_len as u64;
        progress(FlashStage::Verify, checked, image_len);
        last_tick = Instant::now();

        let mut remaining = image_len.saturating_sub(first_len as u64);
        while remaining > 0 {
            let want = remaining.min(BLOCK as u64) as usize;
            let slice = &mut buf[..want];
            read_exact_from(device, slice).map_err(|e| ioerr("read back from device", e))?;
            verify_hash.update(&*slice);
            checked += want as u64;
            remaining -= want as u64;

            if last_tick.elapsed() >= PROGRESS_INTERVAL {
                progress(FlashStage::Verify, checked, image_len);
                last_tick = Instant::now();
            }
        }
        progress(FlashStage::Verify, checked, image_len);

        if verify_hash.finalize() != write_digest {
            return Err("Verification failed: the card does not match the image".to_string());
        }
    }

    progress(FlashStage::Customize, 0, 0);
    customize(device, &first_block, provision)?;

    device
        .seek(SeekFrom::Start(0))
        .map_err(|e| ioerr("seek to block 0", e))?;
    device
        .write_all(&first_block)
        .map_err(|e| ioerr("write block 0", e))?;

    device.sync().map_err(|e| ioerr("final sync", e))?;

    Ok(())
}

fn zero_tail(device: &mut dyn BlockDevice) -> Result<(), String> {
    let end = device
        .seek(SeekFrom::End(0))
        .map_err(|e| ioerr("seek to end of device", e))?;
    if end <= BLOCK as u64 {
        return Ok(());
    }
    device
        .seek(SeekFrom::Start(end - BLOCK as u64))
        .map_err(|e| ioerr("seek to tail of device", e))?;
    device
        .write_all(&vec![0u8; BLOCK])
        .map_err(|e| ioerr("wipe tail of device", e))
}

fn customize(
    device: &mut dyn BlockDevice,
    first_block: &[u8],
    provision: &ProvisionConfig,
) -> Result<(), String> {
    let plan = crate::provision::plan(provision);
    let markers = crate::provision::boot_markers(provision);
    if matches!(plan, ProvisionPlan::Noop) && markers.is_empty() {
        return Ok(());
    }

    let (offset, length) = fat_partition_extent(first_block)?;

    {
        let slice = fscommon::StreamSlice::new(&mut *device, offset, offset + length)
            .map_err(|e| ioerr("slice boot partition", e))?;
        let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new())
            .map_err(|e| ioerr("open FAT filesystem", e))?;
        let root = fs.root_dir();

        match plan {
            ProvisionPlan::Noop => {}
            ProvisionPlan::CloudInit { files } => {
                for f in &files {
                    write_fat_file(&root, f)?;
                }
            }
            ProvisionPlan::FirstRun {
                script,
                cmdline_append,
            } => {
                write_fat_file(&root, &script)?;
                append_cmdline(&root, &cmdline_append)?;
            }
        }
        for f in &markers {
            write_fat_file(&root, f)?;
        }
    }

    device
        .sync()
        .map_err(|e| ioerr("sync after customize", e))?;
    Ok(())
}

fn write_fat_file<T: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'_, T>,
    file: &ProvisionFile,
) -> Result<(), String> {
    let mut fh = root
        .create_file(&file.name)
        .map_err(|e| ioerr(&format!("create {}", file.name), e))?;
    fh.truncate()
        .map_err(|e| ioerr(&format!("truncate {}", file.name), e))?;
    fh.write_all(file.contents.as_bytes())
        .map_err(|e| ioerr(&format!("write {}", file.name), e))?;
    Ok(())
}

fn append_cmdline<T: fatfs::ReadWriteSeek>(
    root: &fatfs::Dir<'_, T>,
    fragment: &str,
) -> Result<(), String> {
    let mut current = String::new();
    if let Ok(mut fh) = root.open_file("cmdline.txt") {
        fh.read_to_string(&mut current)
            .map_err(|e| ioerr("read cmdline.txt", e))?;
    }
    if current.contains(fragment.trim()) {
        return Ok(());
    }

    let trimmed = current.trim_end_matches(['\r', '\n']);
    let updated = format!("{trimmed}{fragment}");

    let mut fh = root
        .create_file("cmdline.txt")
        .map_err(|e| ioerr("create cmdline.txt", e))?;
    fh.truncate()
        .map_err(|e| ioerr("truncate cmdline.txt", e))?;
    fh.write_all(updated.as_bytes())
        .map_err(|e| ioerr("write cmdline.txt", e))?;
    Ok(())
}

fn fat_partition_extent(first_block: &[u8]) -> Result<(u64, u64), String> {
    let mut cursor = Cursor::new(first_block);
    let mbr = MBR::read_from(&mut cursor, SECTOR as u32).map_err(|e| format!("parse MBR: {e}"))?;

    for (_, part) in mbr.iter() {
        if part.is_used() && is_fat(part.sys) {
            let offset = u64::from(part.starting_lba) * SECTOR;
            let length = u64::from(part.sectors) * SECTOR;
            return Ok((offset, length));
        }
    }
    Err("no FAT boot partition found in image MBR".to_string())
}

fn is_fat(sys: u8) -> bool {
    matches!(sys, 0x01 | 0x04 | 0x06 | 0x0b | 0x0c | 0x0e)
}

fn read_full(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match r.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

fn read_exact_from(r: &mut dyn Read, buf: &mut [u8]) -> std::io::Result<()> {
    let n = read_full(r, buf)?;
    if n < buf.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "device returned fewer bytes than the image length",
        ));
    }
    Ok(())
}

fn ioerr(ctx: &str, e: std::io::Error) -> String {
    format!("{ctx}: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InitFormat, WifiConfig};

    const PSK: &str = "f42c6fc52df0ebef9ebb4b90b38a5f902e83fe1b135a70e23aed762e9710a12e";

    struct MemDevice {
        cur: Cursor<Vec<u8>>,
    }

    impl MemDevice {
        fn zeroed(len: usize) -> Self {
            Self {
                cur: Cursor::new(vec![0u8; len]),
            }
        }
        fn into_bytes(self) -> Vec<u8> {
            self.cur.into_inner()
        }
    }

    impl Read for MemDevice {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.cur.read(buf)
        }
    }
    impl Write for MemDevice {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.cur.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.cur.flush()
        }
    }
    impl Seek for MemDevice {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.cur.seek(pos)
        }
    }
    impl BlockDevice for MemDevice {
        fn sync(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    struct FlakyDevice {
        cur: Cursor<Vec<u8>>,
        corrupt_at: u64,
        corrupted: bool,
    }

    impl Read for FlakyDevice {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.cur.read(buf)
        }
    }
    impl Write for FlakyDevice {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            let pos = self.cur.position();
            let n = self.cur.write(buf)?;
            if !self.corrupted && self.corrupt_at >= pos && self.corrupt_at < pos + n as u64 {
                let inner = self.cur.get_mut();
                inner[self.corrupt_at as usize] ^= 0xff;
                self.corrupted = true;
            }
            Ok(n)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.cur.flush()
        }
    }
    impl Seek for FlakyDevice {
        fn seek(&mut self, pos: SeekFrom) -> std::io::Result<u64> {
            self.cur.seek(pos)
        }
    }
    impl BlockDevice for FlakyDevice {
        fn sync(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    const PART_START_SECTOR: u32 = 2048;
    const FAT_SECTORS: u32 = 16 * 1024;
    const IMAGE_SECTORS: u32 = PART_START_SECTOR + FAT_SECTORS + 2048;

    fn build_image() -> Vec<u8> {
        let total_bytes = IMAGE_SECTORS as usize * SECTOR as usize;
        let mut image = vec![0u8; total_bytes];

        let mut cursor = Cursor::new(vec![0u8; total_bytes]);
        let mut mbr =
            MBR::new_from(&mut cursor, SECTOR as u32, [0x12, 0x34, 0x56, 0x78]).expect("new MBR");
        mbr[1] = mbrman::MBRPartitionEntry {
            boot: 0,
            first_chs: mbrman::CHS::empty(),
            sys: 0x0c,
            last_chs: mbrman::CHS::empty(),
            starting_lba: PART_START_SECTOR,
            sectors: FAT_SECTORS,
        };
        let mut mbr_cursor = Cursor::new(vec![0u8; SECTOR as usize]);
        mbr.write_into(&mut mbr_cursor).expect("write MBR");
        let mbr_bytes = mbr_cursor.into_inner();
        image[..SECTOR as usize].copy_from_slice(&mbr_bytes);

        let part_off = PART_START_SECTOR as usize * SECTOR as usize;
        let part_len = FAT_SECTORS as usize * SECTOR as usize;
        {
            let slice = fscommon::StreamSlice::new(
                Cursor::new(&mut image[part_off..part_off + part_len]),
                0,
                part_len as u64,
            )
            .expect("slice for format");
            fatfs::format_volume(
                slice,
                fatfs::FormatVolumeOptions::new().fat_type(fatfs::FatType::Fat32),
            )
            .expect("format FAT");
        }

        {
            let slice = fscommon::StreamSlice::new(
                Cursor::new(&mut image[part_off..part_off + part_len]),
                0,
                part_len as u64,
            )
            .expect("slice for seed");
            let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new()).expect("open seed fs");
            let mut f = fs
                .root_dir()
                .create_file("cmdline.txt")
                .expect("create cmdline");
            f.write_all(b"console=serial0,115200 root=PARTUUID=abcd rootwait")
                .expect("write cmdline");
        }

        image
    }

    fn cloud_init_config() -> ProvisionConfig {
        ProvisionConfig {
            hostname: Some("aircast".into()),
            wifi: Some(WifiConfig {
                ssid: "IEEE".into(),
                password: "password".into(),
                country: "US".into(),
            }),
            tailscale: None,
            access: None,
            encryption: None,
            fight_mode: false,
            init_format: InitFormat::CloudInit,
        }
    }

    fn read_fat_file(image: &[u8], name: &str) -> Option<String> {
        let part_off = PART_START_SECTOR as usize * SECTOR as usize;
        let part_len = FAT_SECTORS as usize * SECTOR as usize;
        let mut owned = image[part_off..part_off + part_len].to_vec();
        let slice = fscommon::StreamSlice::new(Cursor::new(&mut owned), 0, part_len as u64).ok()?;
        let fs = fatfs::FileSystem::new(slice, fatfs::FsOptions::new()).ok()?;
        let mut f = fs.root_dir().open_file(name).ok()?;
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        Some(s)
    }

    #[test]
    fn flash_writes_verifies_and_customizes() {
        let image = build_image();
        let image_len = image.len() as u64;
        let mut device = MemDevice::zeroed(image.len());

        let cfg = cloud_init_config();
        let params = FlashParams {
            image_len,
            verify: true,
        };

        let mut reader = Cursor::new(image.clone());
        flash(&mut device, &mut reader, &params, &cfg, &mut |_, _, _| {})
            .expect("flash should succeed and verify");

        let written = device.into_bytes();

        assert_eq!(
            &written[..SECTOR as usize],
            &image[..SECTOR as usize],
            "MBR on device must match the image"
        );

        let net = read_fat_file(&written, "network-config").expect("network-config exists");
        assert!(net.contains(PSK), "network-config must contain derived PSK");
        assert!(
            !net.contains("password: \"password\""),
            "no plaintext passphrase"
        );

        let user = read_fat_file(&written, "user-data").expect("user-data exists");
        assert!(user.contains("hostname: aircast"));

        assert!(read_fat_file(&written, "meta-data").is_some());
    }

    #[test]
    fn firstrun_appends_cmdline() {
        let image = build_image();
        let image_len = image.len() as u64;
        let mut device = MemDevice::zeroed(image.len());

        let cfg = ProvisionConfig {
            hostname: Some("aircast".into()),
            wifi: Some(WifiConfig {
                ssid: "IEEE".into(),
                password: "password".into(),
                country: "US".into(),
            }),
            tailscale: None,
            access: None,
            encryption: None,
            fight_mode: false,
            init_format: InitFormat::FirstRun,
        };
        let params = FlashParams {
            image_len,
            verify: false,
        };

        let mut reader = Cursor::new(image);
        flash(&mut device, &mut reader, &params, &cfg, &mut |_, _, _| {}).expect("flash firstrun");

        let written = device.into_bytes();
        let script = read_fat_file(&written, "firstrun.sh").expect("firstrun.sh exists");
        assert!(script.contains(&format!("psk={PSK}")));

        let cmdline = read_fat_file(&written, "cmdline.txt").expect("cmdline.txt exists");
        assert!(
            cmdline.contains("console=serial0,115200"),
            "kept original cmdline"
        );
        assert!(
            cmdline.contains("systemd.run=/boot/firstrun.sh"),
            "appended firstrun fragment"
        );
    }

    #[test]
    fn fight_mode_alone_lands_its_flag_on_the_card() {
        let image = build_image();
        let image_len = image.len() as u64;
        let mut device = MemDevice::zeroed(image.len());

        let cfg = ProvisionConfig {
            hostname: None,
            wifi: None,
            tailscale: None,
            access: None,
            encryption: None,
            fight_mode: true,
            init_format: InitFormat::CloudInit,
        };
        let params = FlashParams {
            image_len,
            verify: false,
        };

        let mut reader = Cursor::new(image);
        flash(&mut device, &mut reader, &params, &cfg, &mut |_, _, _| {})
            .expect("flash a fight card");

        let written = device.into_bytes();
        assert_eq!(
            read_fat_file(&written, "aircast-fight-mode").as_deref(),
            Some("")
        );
        assert!(read_fat_file(&written, "user-data").is_none());
    }

    #[test]
    fn flash_wipes_the_tail_of_a_larger_card() {
        let image = build_image();
        let image_len = image.len() as u64;
        let card_len = image.len() + 4 * BLOCK;

        let mut device = MemDevice::zeroed(card_len);
        let last_sector = card_len - SECTOR as usize;
        device.cur.get_mut()[last_sector..last_sector + 8].copy_from_slice(b"EFI PART");

        let cfg = cloud_init_config();
        let params = FlashParams {
            image_len,
            verify: true,
        };

        let mut reader = Cursor::new(image);
        flash(&mut device, &mut reader, &params, &cfg, &mut |_, _, _| {})
            .expect("flash onto an oversized card");

        let written = device.into_bytes();
        assert_eq!(written.len(), card_len, "the card must not grow or shrink");
        assert!(
            written[card_len - BLOCK..].iter().all(|&b| b == 0),
            "a previous card's backup GPT must not survive the flash"
        );
    }

    #[test]
    fn verify_catches_corruption() {
        let image = build_image();
        let image_len = image.len() as u64;

        let corrupt_at = (PART_START_SECTOR as u64 + 100) * SECTOR;
        let mut device = FlakyDevice {
            cur: Cursor::new(vec![0u8; image.len()]),
            corrupt_at,
            corrupted: false,
        };

        let cfg = cloud_init_config();
        let params = FlashParams {
            image_len,
            verify: true,
        };

        let mut reader = Cursor::new(image);
        let err = flash(&mut device, &mut reader, &params, &cfg, &mut |_, _, _| {})
            .expect_err("verify must fail on a corrupted card");
        assert!(err.contains("Verification failed"), "got: {err}");
    }
}
