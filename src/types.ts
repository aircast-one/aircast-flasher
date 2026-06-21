export type Channel = "stable" | "development" | "staging";

export type InitFormat = "cloud-init" | "first-run";

export interface ImageInfo {
  filename: string;
  extension: string;
  size: number;
  download_url: string;
  checksum_url: string;
}

export interface Release {
  version: string;
  prerelease: boolean;
  created_at: string;
  image: ImageInfo;
}

export interface ListReleasesResponse {
  releases: Release[];
}

export interface BlockDevice {
  path: string;
  name: string;
  size: number;
  size_human: string;
  removable: boolean;
  mounted: boolean;
  mount_points: string[];
}

export interface DownloadResult {
  image_path: string;
  checksum: string;
  cached: boolean;
}

export interface DownloadProgress {
  downloaded_bytes: number;
  total_bytes: number;
  percent: number;
  speed_bps: number;
}

export type FlashPhase =
  | "decompressing"
  | "writing"
  | "verifying"
  | "customizing";

export interface FlashProgress {
  phase: FlashPhase;
  bytes_processed: number;
  total_bytes: number;
  percent: number;
}

export interface WifiConfig {
  ssid: string;
  password: string;
  country: string;
}

export interface WifiNetworks {
  current: string | null;
  known: string[];
  country: string | null;
}
