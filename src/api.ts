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
  CloudCard,
  CloudKeyset,
  CloudSignIn,
  CloudSignInStatus,
  BlockDevice,
  Channel,
  DownloadProgress,
  DownloadResult,
  EncryptionConfig,
  FlashProgress,
  InitFormat,
  ListReleasesResponse,
  Settings,
  LocalTailscale,
  SshKeyIdentity,
  SshPublicKey,
  TailscaleConfig,
  TailscaleState,
  WifiConfig,
  WifiNetworks,
} from "./types";

const DOWNLOAD_PROGRESS_EVENT = "flasher:download-progress";
const FLASH_PROGRESS_EVENT = "flasher:flash-progress";

export function listReleases(channel?: Channel): Promise<ListReleasesResponse> {
  return invoke<ListReleasesResponse>("list_releases", {
    channel: channel ?? null,
  });
}

export function listBlockDevices(): Promise<BlockDevice[]> {
  return invoke<BlockDevice[]>("list_block_devices");
}

export function track(
  event: string,
  props: Record<string, unknown> = {},
): void {
  void invoke("track", { event, props }).catch(() => undefined);
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
  encryption: EncryptionConfig | null;
  fightMode: boolean;
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
    encryption: args.encryption,
    fightMode: args.fightMode,
    initFormat: args.initFormat,
  });
}

export function fetchTangAdv(url: string): Promise<string> {
  return invoke<string>("fetch_tang_adv", { url });
}

export function cloudSignInStart(): Promise<CloudSignIn> {
  return invoke<CloudSignIn>("cloud_sign_in_start");
}

export function cloudSignInPoll(token: string): Promise<CloudSignInStatus> {
  return invoke<CloudSignInStatus>("cloud_sign_in_poll", { token });
}

export function cloudAccount(): Promise<string | null> {
  return invoke<string | null>("cloud_account");
}

export function cloudSignOut(): Promise<void> {
  return invoke<void>("cloud_sign_out");
}

export function cloudCreateKeyset(label: string): Promise<CloudKeyset> {
  return invoke<CloudKeyset>("cloud_create_keyset", { label });
}

export function cloudListCards(): Promise<CloudCard[]> {
  return invoke<CloudCard[]>("cloud_list_cards");
}

export const cloudCheckSignIn = (): Promise<void> =>
  cloudListCards().then(() => undefined);

export function cloudMarkLost(id: string): Promise<void> {
  return invoke<void>("cloud_mark_lost", { id });
}

export function saveRecoveryFile(
  defaultName: string,
  contents: string,
): Promise<boolean> {
  return invoke<boolean>("save_recovery_file", { defaultName, contents });
}

export function listWifiNetworks(): Promise<WifiNetworks> {
  return invoke<WifiNetworks>("list_wifi_networks");
}

export function detectSshKeys(): Promise<SshPublicKey[]> {
  return invoke<SshPublicKey[]>("detect_ssh_keys");
}

export async function pickAndReadPublicKey(): Promise<string | null> {
  const path = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "SSH public key", extensions: ["pub"] }],
  });
  if (typeof path !== "string") return null;
  return invoke<string>("read_public_key", { path });
}

export function identifyPublicKey(
  key: string,
): Promise<SshKeyIdentity | null> {
  return invoke<SshKeyIdentity | null>("identify_public_key", { key });
}

export function cancelFlash(): Promise<void> {
  return invoke<void>("cancel_flash");
}

export function probeDevice(url: string): Promise<boolean> {
  return invoke<boolean>("probe_device", { url });
}

export function setDeviceControlServer(
  url: string,
  controlServer: string,
): Promise<void> {
  return invoke<void>("set_device_control_server", { url, controlServer });
}

export function openDeviceWindow(url: string, title: string): Promise<void> {
  return invoke<void>("open_device_window", { url, title });
}

export function probeDevices(urls: string[]): Promise<string[]> {
  return invoke<string[]>("probe_devices", { urls });
}

export function tailscaleLogin(url: string): Promise<string> {
  return invoke<string>("tailscale_login", { url });
}

export function tailscaleStatus(url: string): Promise<TailscaleState> {
  return invoke<TailscaleState>("tailscale_status", { url });
}

export function localTailscale(): Promise<LocalTailscale> {
  return invoke<LocalTailscale>("local_tailscale");
}

export function joinWifi(ssid: string, password: string): Promise<void> {
  return invoke<void>("join_wifi", { ssid, password });
}

export function readSettings(): Promise<Settings> {
  return invoke<Settings>("read_settings");
}

export function updateSettings(patch: Partial<Settings>): Promise<Settings> {
  return invoke<Settings>("update_settings", { patch });
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

export async function checkForUpdate(): Promise<Update | null> {
  if (import.meta.env.DEV) return null;
  try {
    const update = await check();
    return update?.available ? update : null;
  } catch {
    return null;
  }
}

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
