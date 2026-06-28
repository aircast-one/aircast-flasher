//! flasher-core — pure, testable logic shared by the Aircast Flasher app.
//!
//! Deliberately free of I/O, OS calls, and Tauri: it derives the WPA-PSK so the
//! plaintext WiFi passphrase never lands on the card ([`psk`]) and turns a
//! [`ProvisionConfig`] into the concrete first-boot files to drop on the FAT
//! boot partition ([`provision`]). The Tauri app mounts the partition and writes
//! the [`provision::plan`] output.

pub mod device;
pub mod engine;
pub mod model;
pub mod provision;
pub mod psk;

#[cfg(windows)]
pub mod win;

pub use device::AlignedDevice;
pub use engine::{flash, BlockDevice, FlashParams, FlashStage, BLOCK};

pub use model::{InitFormat, ProvisionConfig, TailscaleConfig, WifiConfig};
pub use provision::{plan, ProvisionFile, ProvisionPlan};
