import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";
import type {
  BlockDevice,
  Channel,
  DownloadProgress,
  DownloadResult,
  FlashProgress,
  InitFormat,
  ListReleasesResponse,
  Release,
  WifiConfig,
  WifiNetworks,
} from "./types";

const DOWNLOAD_PROGRESS_EVENT = "flasher:download-progress";
const FLASH_PROGRESS_EVENT = "flasher:flash-progress";

export async function listReleases(channel?: Channel): Promise<Release[]> {
  // Pass null (not "stable") when no channel is chosen, so the backend runs its
  // stable → staging → development fallback instead of pinning to empty stable.
  const response = await invoke<ListReleasesResponse>("list_releases", {
    channel: channel ?? null,
  });
  return response.releases;
}

export function listBlockDevices(): Promise<BlockDevice[]> {
  return invoke<BlockDevice[]>("list_block_devices");
}

export function downloadImage(args: {
  downloadUrl: string;
  checksumUrl: string;
}): Promise<DownloadResult> {
  return invoke<DownloadResult>("download_image", {
    downloadUrl: args.downloadUrl,
    checksumUrl: args.checksumUrl,
  });
}

export function flashImage(args: {
  imagePath: string;
  targetDisk: string;
  wifi: WifiConfig | null;
  hostname: string | null;
  initFormat: InitFormat;
}): Promise<void> {
  return invoke<void>("flash_image", {
    imagePath: args.imagePath,
    targetDisk: args.targetDisk,
    wifi: args.wifi,
    hostname: args.hostname,
    initFormat: args.initFormat,
  });
}

export function listWifiNetworks(): Promise<WifiNetworks> {
  return invoke<WifiNetworks>("list_wifi_networks");
}

export function cancelFlash(): Promise<void> {
  return invoke<void>("cancel_flash");
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

export async function pickLocalImage(): Promise<string | null> {
  const result = await open({
    multiple: false,
    directory: false,
    filters: [{ name: "Image", extensions: ["img", "xz", "gz"] }],
  });
  return result;
}
