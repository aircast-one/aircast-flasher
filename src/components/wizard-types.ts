import type {
  DownloadProgress,
  FlashProgress,
  Release,
  TailscaleConfig,
  WifiConfig,
} from "@/types";

export type SourceKind = "aircast" | "local";

export type WizardStep = 1 | 2 | 3 | 4;

export const STEP = {
  os: 1,
  network: 2,
  storage: 3,
  write: 4,
} as const satisfies Record<string, WizardStep>;

export type FlashProgressState =
  | { phase: "idle" }
  | { phase: "downloading"; progress: DownloadProgress | null }
  | { phase: "flashing"; progress: FlashProgress | null };

export interface FlashVars {
  sourceKind: SourceKind;
  release: Release | null;
  localPath: string | null;
  targetDisk: string;
  wifi: WifiConfig | null;
  hostname: string | null;
  tailscale: TailscaleConfig | null;
}
