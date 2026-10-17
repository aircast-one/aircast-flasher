import type { TangServer } from "@/types";

export const hasUrl = (s: TangServer) => s.url.trim() !== "";
export const isCaptured = (s: TangServer) =>
  s.url.trim() !== "" && s.adv.trim() !== "";

export const serverCount = (p: { servers: TangServer[]; cloud: boolean }) =>
  p.servers.filter(hasUrl).length + (p.cloud ? 1 : 0);

export function serversReady(p: {
  servers: TangServer[];
  threshold: number;
  cloud: boolean;
}): boolean {
  const total = serverCount(p);
  return (
    total > 0 &&
    p.servers.filter(hasUrl).every(isCaptured) &&
    p.threshold >= 1 &&
    p.threshold <= total
  );
}

export function encryptionIncomplete(p: {
  enabled: boolean;
  servers: TangServer[];
  threshold: number;
  cloud: boolean;
  recoverySaved: boolean;
}): boolean {
  return p.enabled && !(serversReady(p) && p.recoverySaved);
}
