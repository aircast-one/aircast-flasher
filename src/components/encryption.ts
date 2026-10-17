import type { TangServer } from "@/types";

export const hasUrl = (s: TangServer) => s.url.trim() !== "";
export const isCaptured = (s: TangServer) =>
  s.url.trim() !== "" && s.adv.trim() !== "";

/** Encryption is set up but an input needed to enrol it is still missing. */
export function encryptionIncomplete(p: {
  enabled: boolean;
  servers: TangServer[];
  threshold: number;
  recoverySaved: boolean;
}): boolean {
  if (!p.enabled) return false;
  const urls = p.servers.filter(hasUrl);
  const captured = p.servers.filter(isCaptured);
  return (
    urls.length === 0 ||
    captured.length !== urls.length ||
    p.threshold < 1 ||
    p.threshold > urls.length ||
    !p.recoverySaved
  );
}
