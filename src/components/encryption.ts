import { gte, valid } from "semver";
import type { TangServer } from "@/types";
import type { FieldProblem } from "@/components/step-network.validation";

export const ENCRYPTION_MIN_IMAGE = "0.3.6";

export const releaseSupportsEncryption = (version: string) =>
  valid(version) !== null && gte(version, `${ENCRYPTION_MIN_IMAGE}-0`);

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

function keyServerProblems(p: {
  servers: TangServer[];
  threshold: number;
  cloud: boolean;
}): FieldProblem[] {
  const total = serverCount(p);
  const untested = p.servers.findIndex((s) => hasUrl(s) && !isCaptured(s));
  return total === 0
    ? [
        {
          field: "tang-server-0",
          message:
            "Add a key server for disk encryption, or turn on unlock through Aircast Cloud",
        },
      ]
    : untested >= 0
      ? [
          {
            field: `tang-server-${untested}`,
            message: "Test the connection to your key server",
          },
        ]
      : p.threshold < 1 || p.threshold > total
        ? [
            {
              field: "threshold",
              message: `Set the key servers needed to unlock between 1 and ${total}`,
            },
          ]
        : [];
}

export function encryptionProblems(p: {
  enabled: boolean;
  servers: TangServer[];
  threshold: number;
  cloud: boolean;
  recoverySaved: boolean;
}): FieldProblem[] {
  if (!p.enabled) return [];
  return [
    ...keyServerProblems(p),
    ...(p.recoverySaved
      ? []
      : [
          {
            field: "recovery-key",
            message:
              "Save the recovery key — download it, or confirm you saved it elsewhere",
          },
        ]),
  ];
}
