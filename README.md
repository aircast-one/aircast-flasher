# Aircast Flasher

A cross-platform desktop app (Tauri + React) that flashes the Aircast Raspberry Pi
image to an SD card and provisions **WiFi** + **hostname** in one step — a focused,
re-branded alternative to Raspberry Pi Imager locked to the Aircast image.

Built on the proven flasher in `aircast-web` (Authorization Services + `dd`,
`releases.json`, hardened input validation), specialized for WiFi/hostname setup.

## How it works

```
list_releases ─► download_image ─► flash_image ─► provision_device
 (releases.json)  (stream+sha256)   (decompress     (mount boot part,
                                     + dd as root)    write cloud-init/firstrun,
                                                      eject)
```

- **Image source** — `downloads.aircast.one/images/releases.json` (channels:
  `stable` / `development` / `staging`), or a local `.img`/`.img.xz`/`.img.gz`.
- **Privileged write** — on macOS, one native auth prompt (Touch ID) via
  Authorization Services, then `dd of=/dev/rdiskN bs=1m` runs as root with the
  decompressed image streamed to its stdin (live progress + cancel). On Linux,
  `pkexec dd`. **No separate helper binary** to sign or bundle.
- **Provisioning** — after writing, the boot partition is mounted and the WiFi
  PSK + hostname are written as cloud-init (`user-data` + `network-config`) or
  legacy `firstrun.sh`, then the card is ejected. The plaintext WiFi passphrase
  is never written — only the derived WPA-PSK.

## Layout

```
crates/flasher-core/    pure, tested logic (no I/O):
  src/psk.rs              WPA-PSK derivation (PBKDF2-HMAC-SHA1)
  src/provision.rs        ProvisionPlan: cloud-init / firstrun file contents
  src/model.rs            WifiConfig · ProvisionConfig · InitFormat
src-tauri/
  src/flasher.rs          commands: releases, devices, download, flash, provision, cancel
  src/macos_auth.rs       macOS Authorization Services (run dd as root)
  src/main.rs             app builder, command registration
src/                    React + TypeScript UI
```

## Develop

Requires Rust, Node, and the Tauri prerequisites. Uses **npm**.

```sh
npm install
npm run tauri dev
```

### Tests

```sh
cargo test              # flasher-core (psk/provision) + flasher.rs (validation)
npm run build           # tsc + vite typecheck/build
```

## Safety

`list_block_devices` only ever returns removable, non-system disks ≤ 2 TB (the
system disk is excluded via the `Internal` flag on macOS / `rm` + mountpoint on
Linux). `flash_image` re-validates the target is still a removable device before
writing, and `validate_disk_path` rejects anything that isn't `/dev/diskN`
(macOS) / `/dev/sdX` (Linux) — closing command-injection paths into `dd`/`diskutil`.

## TODO before shipping

- **Confirm `initFormat`** — the UI defaults to `cloud-init`; it must match the
  image base (Trixie → cloud-init, Bookworm → first-run) or provisioning silently
  no-ops on first boot.
- **Code-sign + notarize** (macOS Developer ID + notarization; Windows
  Authenticode/EV). Note: `macos_auth.rs` uses `AuthorizationExecuteWithPrivileges`
  (deprecated since 10.7); the modern path is an `SMAppService` privileged helper,
  which requires a real Developer ID — see the TODO in that file.
- **Windows write support** — not implemented (mirrors aircast-web, which is
  macOS + Linux). Add a `\\.\PhysicalDrive` path if needed.
- **Updater** — wire `tauri-plugin-updater` like aircast-web (pubkey + release
  endpoint) once releases are published.
