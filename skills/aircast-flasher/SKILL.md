---
name: aircast-flasher
description: Flash Aircast OS onto SD cards for Raspberry Pi drones and manage the fleet with aircast-flasher-cli or its MCP tools. Use this whenever someone wants to flash, write, burn or provision an SD card for an Aircast drone or Raspberry Pi; set a card's WiFi, hostname, SSH key, Tailscale or Headscale key, disk encryption (Tang / Aircast Cloud unlock) or fight mode; download an Aircast OS image; find a freshly flashed drone on the network; list drones on the tailnet or sign one in to Tailscale; or list Aircast Cloud cards and mark a lost one — even if they only say "set up a new drone card" or "prep falcon-02".
---

# Aircast Flasher

`aircast-flasher-cli` does everything the Aircast Flasher app does, from a
terminal: list releases and SD cards, write a configured card, then find the
drone once it boots. `aircast-flasher-cli mcp` exposes the same operations as
MCP tools. Both write the same cards as the app, with the same image cache and
the same elevated write helper.

## Pick the interface

- If the `aircast-flasher` MCP server is connected (tools named `flash_card`,
  `list_cards`, …), use those tools: they return JSON and stream progress.
- Otherwise run `aircast-flasher-cli` with `--json` and parse the output.
- Find the binary in this order: `aircast-flasher-cli` on PATH; an existing
  build in an aircast-flasher checkout (`target/release/` or `target/debug/`);
  the latest release (`gh release download -R aircast-one/aircast-flasher -p
  'aircast-flasher-cli_*'`, then unpack the archive for this platform); or build
  it with `just cli` in the repo.

## Quick questions

"Is a card in?", "what's online?", "which releases are there?" need no
flashing and no confirmation — answer them straight from the read-only
commands:

| Command (`--json`) | MCP | Output |
|---|---|---|
| `cards` | `list_cards` | array of `{path, name, size, size_human, mounted, mount_points}`; empty means no card inserted |
| `devices` | `list_devices` | `{tailnet, devices: [{name, address, ip}], offline: [names], notAnswering: [names]}` |
| `releases` | `list_releases` | `{releases: [{version, prerelease, created_at, image}], min_card_bytes}` |
| `find NAME` | `find_device` | `{address}` — `null` until the drone has booted |

`devices` lists drones that answer as Aircast on this computer's tailnet.
`offline` names Linux machines on the tailnet that are offline — they may be
drones, servers or other Pis, so call them "offline machines", not drones.
`notAnswering` names online Linux machines that don't answer as Aircast
(a VPS, a Pi without Aircast, or a drone still booting).

## Flashing a card

Flashing erases the whole card, and the wrong disk can be someone's data, so
the disk always comes from what the computer reports — never from a guess.

1. **Find the card.** `aircast-flasher-cli cards --json` (MCP `list_cards`).
   Only removable SD cards are listed. If none, ask the person to insert one;
   if several, ask which, showing each `path`, `name` and `size_human`.
2. **Pick the image.** Default is the newest stable Aircast OS for a
   Raspberry Pi (`releases` shows them; prereleases are marked). For another
   board pass `--board orangepi5` or `--board rock-5b` (same flag on
   `releases` and `download`; ask which board when the person hasn't said and
   it isn't a Pi). Those two have no built-in WiFi: leave the WiFi flags out
   unless the person has a USB WiFi adapter (the CLI prints a note if WiFi is
   set anyway). Use `--release vX.Y.Z` for a specific
   version, `--channel staging|development` for other channels, or `--image`
   for a local `.img`, `.img.xz` or `.img.gz`. Fight mode and disk encryption
   need Aircast OS v0.3.6 or newer — older images ignore them, so the CLI
   refuses. If the newest stable is older and the person wants either, offer
   the newest release that qualifies and say when it is a prerelease.
3. **Gather the settings** the person asked for (table below). Leave out what
   they didn't mention — every option has a sensible default. If something
   needed is missing (a WiFi password, which card, a tailnet key for fight
   mode, a beta image), ask for all of it in one message rather than one
   question at a time.
4. **Confirm before erasing.** Tell the person which card will be erased
   (path, name, size) and what will be written, and get a clear yes for that
   disk. Only then pass `--yes` (CLI) or set `confirmErase` to the disk path
   (MCP). The CLI refuses to erase without `--yes` when there's no terminal,
   so the confirmation has to happen in the conversation.
5. **Flash.** Writing needs administrator rights: a password / Touch ID dialog
   appears on the person's screen unless the CLI runs as root (e.g. `sudo`).
   Tell them to approve it. A flash takes a few minutes (download, write,
   verify, configure); the image is cached after the first download.
6. **Hand over.** Report the result: the card is ready to go into the drone.
   If a hostname was set, `find <hostname>` locates the drone once it has
   booted (a few minutes after first power-on).

### Settings

| Want | CLI | MCP `flash_card` field |
|---|---|---|
| Hostname | `--hostname falcon-01` | `hostname` |
| WiFi | `--wifi-ssid NAME` + `AIRCAST_WIFI_PASSWORD` env, `--wifi-country GB` | `wifi: {ssid, password, country}` |
| Ethernet / cellular only | leave WiFi out | leave `wifi` out |
| SSH key | `--ssh-key ~/.ssh/id_ed25519.pub` or the key text | `ssh: {mode: "key", key}` |
| SSH password | `AIRCAST_DEVICE_PASSWORD` env (8+ chars) | `ssh: {mode: "password", password}` |
| SSH off | `--no-ssh` | `ssh: {mode: "off"}` |
| Tailscale | `AIRCAST_TAILSCALE_KEY` env | `tailscale: {authKey}` |
| Headscale | `AIRCAST_TAILSCALE_KEY` + `--control-server https://hs.example.com` | `tailscale: {authKey, controlServer}` |
| Disk encryption | `--tang http://10.0.0.5:7500` (repeatable), `--threshold N`, `--cloud-unlock`, `--recovery-file PATH` | `encryption: {tangServers, threshold, cloudUnlock, recoveryFile}` |
| Fight mode | `--fight` | `fightMode: true` |

Pass secrets (WiFi password, device password, tailnet key) through the
environment variables rather than as arguments, so they stay out of shell
history and process listings.

Defaults worth knowing:
- No SSH option: the only key in `~/.ssh` is used; with none or several, the
  image's default login (pi / raspberry) stays on. Say which happened.
- No WiFi country: this computer's country, else `US`.
- Hostnames: lowercase letters, numbers and hyphens.
- WiFi passwords are 8–63 characters. An empty password writes an **open**
  network, so when the person names a WiFi network without its password, ask
  for it — only leave it empty when they say the network is open.

Rules the CLI enforces, so check them before calling:
- **Fight mode** (no logs or video on the drone, reachable only over the
  tailnet with an SSH key) needs a tailnet key, Aircast OS v0.3.6+, and an SSH
  key or SSH turned off — never a password, and never the image's default
  login. The only key in `~/.ssh` counts; with none or several, pass
  `--ssh-key` (or `--no-ssh`) explicitly.
- **Encryption** needs Aircast OS v0.3.6+, at least one key server (a Tang URL or
  `--cloud-unlock`), a threshold no higher than the server count, and a
  recovery file path that doesn't exist yet. The recovery key is written there
  before the card is, and it is the only copy — tell the person to keep it
  somewhere safe. Cloud unlock needs an Aircast Cloud sign-in first.

When something is missing, the CLI lists every problem at once
("Fix this to continue: …"). Fix them all and retry rather than one at a time.

## After the flash

- `aircast-flasher-cli find falcon-01` (MCP `find_device`) probes
  `falcon-01.local` and the drone's own hotspot (`http://10.42.0.1`). Nothing
  found right after power-on is normal; it restarts once on first boot.
- `device status falcon-01.local` (MCP `device_status`) says whether it's on a
  tailnet. `device login falcon-01.local [--control-server URL]`
  (MCP `device_tailscale_login`) starts Tailscale sign-in on the drone and
  returns a URL the person must open to approve it.
- `devices` (MCP `list_devices`) lists Aircast drones answering on this
  computer's tailnet. It needs Tailscale installed and signed in here; the
  error says which is missing.

## Aircast Cloud

Cloud unlock lets an encrypted card unlock anywhere it gets online, until it
is marked lost.

- Sign in: `cloud login` opens the browser and waits for approval. Over MCP,
  `cloud_sign_in` returns the URL (and opens it), then `cloud_sign_in_finish`
  with the returned token waits up to 5 minutes. The session is kept on this
  computer and shared by the CLI and MCP.
- `cloud cards` / `cloud_list_cards`: each card's id, label, last unlock and
  whether it's lost.
- `cloud lost <id> --yes` / `cloud_mark_lost` (with `confirmLabel` set to the
  card's label) — permanent: the card never unlocks again. Only do this when
  the person has said that specific card is lost or stolen.
- "Signed out of Aircast Cloud — sign in again" means the session expired; run
  the sign-in again.

## Errors you'll meet

| Message | Meaning |
|---|---|
| `… isn't a removable SD card on this computer` | Wrong path or the card was removed — run `cards` again |
| `Refusing to erase … without a prompt — pass --yes` | Confirm the disk with the person, then add `--yes` |
| `Card too small` | The image needs a bigger card (about 4 GB minimum) |
| `Elevated flash failed (authorization declined or write error)` | The admin dialog was dismissed, or the card failed — retry, and try another card if it repeats |
| `Tailscale isn't signed in on this computer` | `devices` needs Tailscale running here |
