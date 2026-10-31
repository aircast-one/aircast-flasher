export type Channel = "stable" | "development" | "staging";

export type InitFormat = "cloud-init" | "first-run";

export interface ImageInfo {
  filename: string;
  extension: string;
  size: number;
  uncompressed_size: number | null;
  download_url: string;
  checksum_url: string;
}

export interface Release {
  version: string;
  prerelease: boolean;
  created_at: string;
  image: ImageInfo;
}

export interface Board {
  id: string;
  name: string;
  maker: string;
  detail: string;
  builtinWifi: boolean;
  beta: boolean;
  tested: boolean;
}

export interface ListReleasesResponse {
  releases: Release[];
  min_card_bytes: number;
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
  speed_bps: number;
}

export interface WifiConfig {
  ssid: string;
  password: string;
  country: string;
}

export interface TailscaleConfig {
  controlServer: string;
  authKey: string;
}

export interface TailnetPeer {
  hostName: string;
  dnsName: string;
  os: string;
  online: boolean;
  ip: string;
  sameTailnet: boolean;
}

export interface LocalTailscale {
  installed: boolean;
  running: boolean;
  name: string;
  tailnet: string;
  magicDnsSuffix: string;
  controlUrl: string;
  selfHosted: boolean;
  peers: TailnetPeer[];
}

export interface TailscaleState {
  connected: boolean;
  name: string;
}

export type SshMode = "key-only" | "password" | "disabled";

export type AccessConfig =
  | { ssh: "key-only"; authorizedKey: string }
  | { ssh: "password"; password: string }
  | { ssh: "disabled" };

export interface TangServer {
  url: string;
  adv: string;
}

export interface EncryptionConfig {
  tangServers: TangServer[];
  threshold: number;
  recoveryKey: string;
}

export interface CloudSignIn {
  token: string;
  authUrl: string;
}

export type CloudSignInStatus =
  | { status: "pending" }
  | { status: "signedIn"; email: string };

export interface CloudKeyset {
  id: string;
  url: string;
  advertisement: unknown;
}

export interface CloudCard {
  id: string;
  label: string;
  createdAt: string;
  lostAt: string | null;
  lastUnlockAt: string | null;
  lastUnlockIp: string | null;
}

export interface WifiNetworks {
  current: string | null;
  known: string[];
  country: string | null;
}

export interface Settings {
  ssid: string | null;
  hostname: string | null;
  wifiPassword: string;
  controlServer: string;
  authorizedKey: string | null;
  noWifi: boolean;
  telemetry: boolean | null;
  installId: string | null;
  flashedHostnames: string[];
  tangUrls: string[];
  tangThreshold: number;
  tangCloud: boolean;
  authKey: string;
  remoteAccess: boolean | null;
  selfHosted: boolean;
  sshMode: SshMode | null;
  devicePassword: string;
  fightMode: boolean;
  encryption: boolean;
  useLocalImage: boolean;
  localImagePath: string | null;
  board: string | null;
}

export interface SshKeyIdentity {
  algorithm: string;
  comment: string;
  fingerprint: string;
}

export interface SshPublicKey {
  label: string;
  contents: string;
}
