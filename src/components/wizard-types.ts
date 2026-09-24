import type {
  AccessConfig,
  DownloadProgress,
  FlashProgress,
  Release,
  TailscaleConfig,
  WifiConfig,
} from "@/types";

export type SourceKind = "aircast" | "local";

export type WizardStep = 1 | 2 | 3 | 4 | 5;

export const STEP = {
  os: 1,
  network: 2,
  access: 3,
  storage: 4,
  write: 5,
} as const satisfies Record<string, WizardStep>;

export type FlashProgressState =
  | { phase: "idle" }
  | { phase: "downloading"; progress: DownloadProgress | null }
  | { phase: "flashing"; progress: FlashProgress | null };

export interface FlashVars {
  jobId: string;
  sourceKind: SourceKind;
  release: Release | null;
  localPath: string | null;
  targetDisk: string;
  wifi: WifiConfig | null;
  hostname: string;
  tailscale: TailscaleConfig | null;
  access: AccessConfig | null;
}
