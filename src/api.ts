import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import {
  check,
  type DownloadEvent,
  type Update,
} from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import type {
  AccessConfig,
  BlockDevice,
  Channel,
  DownloadProgress,
  DownloadResult,
  FlashProgress,
  InitFormat,
  ListReleasesResponse,
  Settings,
  SshPublicKey,
  TailscaleConfig,
  WifiConfig,
  WifiNetworks,
} from "./types";

const DOWNLOAD_PROGRESS_EVENT = "flasher:download-progress";
const FLASH_PROGRESS_EVENT = "flasher:flash-progress";

export function listReleases(channel?: Channel): Promise<ListReleasesResponse> {
  // Pass null (not "stable") when no channel is chosen, so the backend runs its
  // stable → staging → development fallback instead of pinning to empty stable.
  return invoke<ListReleasesResponse>("list_releases", {
    channel: channel ?? null,
  });
}

export function listBlockDevices(): Promise<BlockDevice[]> {
  return invoke<BlockDevice[]>("list_block_devices");
}

export function downloadImage(args: {
  jobId: string;
  downloadUrl: string;
  checksumUrl: string;
}): Promise<DownloadResult> {
  return invoke<DownloadResult>("download_image", {
    jobId: args.jobId,
    downloadUrl: args.downloadUrl,
    checksumUrl: args.checksumUrl,
  });
}

export function flashImage(args: {
  jobId: string;
  imagePath: string;
  targetDisk: string;
  wifi: WifiConfig | null;
  hostname: string | null;
  tailscale: TailscaleConfig | null;
  access: AccessConfig | null;
  initFormat: InitFormat;
}): Promise<void> {
  return invoke<void>("flash_image", {
    jobId: args.jobId,
    imagePath: args.imagePath,
    targetDisk: args.targetDisk,
    wifi: args.wifi,
    hostname: args.hostname,
    tailscale: args.tailscale,
    access: args.access,
    initFormat: args.initFormat,
  });
}

export function listWifiNetworks(): Promise<WifiNetworks> {
  return invoke<WifiNetworks>("list_wifi_networks");
}

export function detectSshKeys(): Promise<SshPublicKey[]> {
  return invoke<SshPublicKey[]>("detect_ssh_keys");
}

// Open a file picker for a .pub key and return its trimmed contents, or null if
// the user cancelled.
export async function pickAndReadPublicKey(): Promise<string | null> {
  const path = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "SSH public key", extensions: ["pub"] }],
  });
  if (typeof path !== "string") return null;
  return invoke<string>("read_public_key", { path });
}

export function cancelFlash(): Promise<void> {
  return invoke<void>("cancel_flash");
}

export function readSettings(): Promise<Settings> {
  return invoke<Settings>("read_settings");
}

export function writeSettings(settings: Settings): Promise<void> {
  return invoke<void>("write_settings", { settings });
}

export function revealEventLog(): Promise<void> {
  return invoke<void>("reveal_event_log");
}

export function onDownloadProgress(
  handler: (progress: DownloadProgress) => void,
): Promise<UnlistenFn> {
  return listen<DownloadProgress>(DOWNLOAD_PROGRESS_EVENT, (event) =>
    handler(event.payload),
  );
}

export function onFlashProgress(
  handler: (progress: FlashProgress) => void,
): Promise<UnlistenFn> {
  return listen<FlashProgress>(FLASH_PROGRESS_EVENT, (event) =>
    handler(event.payload),
  );
}

// Ask the updater endpoint whether a newer build is available. Returns the
// pending Update (with `.version`) or null when up-to-date. Swallows every
// error — the endpoint 404s until the first release ships, and dev builds have
// no updater configured, neither of which should surface to the user.
export async function checkForUpdate(): Promise<Update | null> {
  try {
    const update = await check();
    return update?.available ? update : null;
  } catch {
    return null;
  }
}

// Download + install the pending update, then restart into the new version.
// The caller gets the raw download events so it can show progress; a failed
// download must surface to the user, so nothing is caught here.
export async function installUpdate(
  update: Update,
  onEvent: (event: DownloadEvent) => void,
): Promise<void> {
  await update.downloadAndInstall(onEvent);
  await relaunch();
}

export async function pickLocalImage(): Promise<string | null> {
  const result = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Image", extensions: ["img", "xz", "gz"] }],
  });
  return result;
}
