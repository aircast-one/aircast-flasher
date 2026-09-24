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
- **Progress** — download and write both report a rate over the same
  one-second window (`SPEED_WINDOW`). The write is the longest wait in the app,
  so a bar alone would answer "is it moving" but not "how long"; the meter
  restarts on a phase change, since write and verify run at different speeds
  over the same byte count.
- **Privileged write** — on macOS, one native auth prompt (Touch ID) via
  Authorization Services, then `dd of=/dev/rdiskN bs=1m` runs as root with the
  decompressed image streamed to its stdin (live progress + cancel). On Linux,
  `pkexec dd`. **No separate helper binary** to sign or bundle.
- **Steps** — Operating system → **Network** (WiFi, hostname: whether it comes
  online and what it's called) → **Access** (remote access, device access: both
  optional, how you get in) → Storage → Write. Split because one screen was
  answering four questions with two of them optional, so the optional half
  inflated the mandatory half.
- **The default hostname is the next free number** — `aircast-01`, then
  `aircast-02`, chosen by checking the tailnet rather than by rolling random
  hex. The hex existed only because the app could not see the fleet; it can
  now, so the name says which device this is and cannot collide with one that
  already exists. A gap is filled rather than skipped. Two sources say a name is
  taken: the tailnet, and `flashedHostnames` in settings — the names this app
  has written before, which is what keeps a batch flashed *offline* counting up
  when there is no tailnet to consult. "Flash another" advances from whatever
  the operator typed (`falcon-01` -> `falcon-02`), skipping anything taken, so
  their naming survives rather than being replaced by a default.
- **The SSH key is named before it locks you out** — an authorized key turns
  password login off, and the field truncates before the `user@host` comment,
  so `identify_public_key` prints the comment, the algorithm, and the SHA256
  fingerprint `ssh-keygen -lf` prints. A wrong key here costs a re-flash; the
  fingerprint is the only way to check it is the key you hold.
- **Provisioning** — after writing, the boot partition is mounted and the WiFi
  PSK + hostname are written as cloud-init (`user-data` + `network-config`) or
  legacy `firstrun.sh`, then the card is ejected. The plaintext WiFi passphrase
  is never written — only the derived WPA-PSK.
- **Remote access (optional)** — paste a pre-auth key and it is provisioned into
  aircastd's store, so the device joins your tailnet on first boot with no
  sign-in. Getting that key needs a Tailscale account, so the step checks this
  computer with `local_tailscale` (`status --json` via the per-platform install
  paths — it is not on `PATH` after a Mac App Store install) and says which of
  three situations you're in: signed in (naming the account the key will come
  from), installed but signed out, or not installed — offering **Get a key**
  and, when missing, **Get Tailscale**.
- **Remote access after boot** — leave the key blank and the device can sign
  itself in instead. Once found on the LAN, `tailscale_login` POSTs to
  aircastd's `/api/tailscale/connection`, which starts the headless tsnet login
  and hands back a `login.tailscale.com` URL; the flasher opens it in the system
  browser and then polls `tailscale_status` until the device reports the join.
  Offered only over the LAN — on the device's own hotspot neither side has
  internet, so the screen says to put the device on WiFi first.
- **The tailnet itself** — `tailscale status --json` also returns every peer,
  so `local_tailscale` reports them (shared-in devices are kept but flagged;
  ones with no `DNSName` are dropped). That powers three things: a hostname
  collision warning before the card is written (a name already on your tailnet
  gets a `-1` suffix from Tailscale and the promised address drifts), a
  join confirmation on the Write screen that works after the device has left
  the LAN — the only way to confirm the pre-auth-key path — and the **Devices**
  view. The join confirmation compares against a
  baseline of the peers present when the screen mounted: matching on hostname
  alone reports a machine that has been on the tailnet for months, which is
  precisely the name the collision warning flagged.
- **Devices** — a sidebar view, available at launch rather than after a flash.
  `probe_devices` hits `/healthz` on every online Linux peer in parallel, so the
  list is what is actually running aircastd, not what happens to run Linux (a
  tailnet's Linux peers include servers and stray Pis). Each row opens the
  device's own page over the tailnet, from anywhere — in an app window, not the
  system browser, so managing a device never leaves the app. It is a native
  webview rather than an iframe because the app's origin is a secure context and
  an embedded `http://` frame is mixed active content the webviews block
  inconsistently across platforms; the device page inherits no Tauri APIs, since
  the capability is scoped to the `main` window. Links that are genuinely
  external — the Tailscale keys page, docs, QGC — still open in the real
  browser, deliberately: Google refuses OAuth sign-in inside an embedded
  webview.

- **Headscale** — `status --json` does not carry the control server, so
  `tailscale debug prefs` supplies `ControlURL`. Anything that is not
  Tailscale's own server switches the whole step: the form opens on the
  self-hosted branch with the detected server filled in, the key help drops the
  Tailscale admin link — Headscale keys come from `headscale preauthkeys
  create`, shown with a copy button — and the post-boot sign-in PUTs the control
  server to the device first, so a self-hosted shop's device never tries to sign
  in to Tailscale.

  Remote access is on **iff a pre-auth key is present**: a control server alone
  writes nothing, which is why prefilling it raises no error. The only guard
  left is a key with no server while self-hosted, where the key would go to
  Tailscale instead.

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

## Settings

WiFi network + passphrase, hostname, control server, authorized SSH key and the
list of hostnames already written are saved to
`<config dir>/one.aircast.flasher/settings.json` (owner-only, 0600 — it holds
the passphrase) via the `read_settings` / `write_settings` commands, rather
than in webview localStorage. The written-hostname list is capped at the most
recent 200.

`null` means "never set" and is distinct from `""` — that is what lets a
cleared SSID or hostname stay cleared instead of snapping back to the detected
network or a generated name.

## Tests

```
just test    # Rust (flasher-core + app), frontend units, release-script units
just e2e     # the wizard driven through the real app over WebDriver
just check   # both
```

`just e2e` is a **pre-push gate, run locally**, not a CI step — run it (or
`just check`) before pushing. On GitHub-hosted runners the WebDriver automation
session is handed a blank webview instead of the app's, so the suite cannot
reach the wizard there:

| Environment | Result |
|---|---|
| macOS desktop | 10/10 in <1s |
| ubuntu-22.04 container, embedded driver | `about:blank`, no asset protocol |
| ubuntu-22.04 container, tauri-driver | same, with `TAURI_WEBVIEW_AUTOMATION=true` verified in the app process |
| macos-latest runner | same, on a commit that passes locally |

The app is not at fault: in the same container it renders at 1000x680 with
healthy `WebKitWebProcess`/`WebKitNetworkProcess`, and an 8s artificial startup
delay does not reproduce the failure locally, ruling out a launch race. What is
left is the automation handover inside wry/tauri-driver, and the runner
environment. CI runs the build, the Rust tests, the frontend units and the
release-script units on Linux.

## Diagnostics

Every run writes wide events (a JSON object per line) to a local log. The
**Show diagnostics** button on a failed flash reveals the file:

| OS | Path |
|----|------|
| macOS | `~/Library/Logs/one.aircast.flasher/events.jsonl` |
| Linux | `$XDG_DATA_HOME/one.aircast.flasher/logs/events.jsonl` (usually `~/.local/share/…`) |
| Windows | `%LOCALAPPDATA%\one.aircast.flasher\logs\events.jsonl` |

One line per event. `download` and `flash` are correlated by `job_id`:

```json
{"event":"flash","job_id":"5f2…","app_version":"0.1.1","os":"macos","arch":"aarch64",
 "outcome":"failed","duration_ms":91000,"error":"write to device: Input/output error",
 "wifi":true,"hostname":true,"ssh":"password","init_format":"cloud-init",
 "image_bytes":3800000000,"compressed":true,"verified":false,"failed_at":"write",
 "decompress_ms":21000,"write_ms":70000,"write_mbps":54.3}
```

Recorded: outcome, the phase a failure happened in, per-phase durations, write
throughput, image size, and *whether* each setting was used. Never recorded: the
SSID, the hostname, the WiFi passphrase, the device password, or the SSH key —
`describe_config` maps configuration to booleans and fixed labels, so a secret cannot reach the log by construction
(`telemetry.rs` tests assert this). The log rolls over at 2 MiB.

The events cover the whole run, not just the destructive part — a flash-only log
cannot tell "no card was ever detected" from "never tried":

| Event | Answers |
|---|---|
| `app_start` | how many installs run each version |
| `consent` | the opt-in itself, and whether a build's sink works at all |
| `step_view` | which step operators reach, and where the ones who never flash stop |
| `devices_listed` | whether a card was detected at all, and how many |
| `releases_failed` | the catalog is unreachable, before anyone reports it |
| `update_check` | whether an update was offered — a stale fleet looks identical to a broken endpoint without it |
| `update_install` | updates that fail to apply |
| `download` / `flash` | the existing wide events, with `job_id`, phases and `diag` |
| `crash` | panics, with message and location |

### Shipping them

Opt-in, off until the operator answers the consent prompt (and reversible from
**Diagnostics on/off** in the sidebar). Turning it on mints a random per-install
id; turning it off drops it. Until then, and in any build without a
`POSTHOG_KEY`, events only ever reach the local file.

Sink is PostHog (`/batch/`, project key baked in at build time from the
`POSTHOG_KEY` secret). The log is the queue: every send starts at a byte offset
kept in `events.sent` beside the log and advances it only on a `2xx`, so a run
that happened offline or died mid-flush ships on the next launch, and nothing is
sent twice. Events written *before* consent are never sent — opting in marks the
existing log as already shipped.

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

## License

GPL-3.0-only. See `LICENSE`. Forks and derived flashers must publish their
source under the same terms.

This repository is the desktop flasher only: it downloads the published
Aircast image, writes network, hostname and access settings onto the card, and
verifies the write. The image itself, `aircastd`, and the Aircast cloud are
separate, proprietary components and are not covered by this license.
