import type { TangServer } from "@/types";

export const hasUrl = (s: TangServer) => s.url.trim() !== "";
export const isCaptured = (s: TangServer) =>
  s.url.trim() !== "" && s.adv.trim() !== "";

export function serversReady(p: {
  servers: TangServer[];
  threshold: number;
}): boolean {
  const urls = p.servers.filter(hasUrl);
  return (
    urls.length > 0 &&
    urls.every(isCaptured) &&
    p.threshold >= 1 &&
    p.threshold <= urls.length
  );
}

export function encryptionIncomplete(p: {
  enabled: boolean;
  servers: TangServer[];
  threshold: number;
  recoverySaved: boolean;
}): boolean {
  return p.enabled && !(serversReady(p) && p.recoverySaved);
}
