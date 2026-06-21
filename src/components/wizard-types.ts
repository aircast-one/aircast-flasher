import type { DownloadProgress, FlashProgress, Release, WifiConfig } from "@/types";

export type SourceKind = "aircast" | "local";

export type WizardStep = 1 | 2 | 3 | 4;

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
}
