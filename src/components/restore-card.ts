import type { BackupCard, Settings, SshMode, TailnetPeer } from "@/types";

export const cardPreset = (card: BackupCard): Partial<Settings> => ({
  fightMode: card.fightMode,
  ...(card.fightMode ? { remoteAccess: true } : {}),
  ...(card.passwordLogin ? { sshMode: "password" as const } : {}),
  encryption: card.encryption !== null,
  ...(card.encryption !== null && card.encryption.threshold > 0
    ? {
        tangUrls: card.encryption.tangUrls,
        tangThreshold: card.encryption.threshold,
        tangCloud: card.encryption.cloud,
      }
    : {}),
  selfHosted: card.controlServer !== "",
  ...(card.controlServer === "" ? {} : { controlServer: card.controlServer }),
});

export const rejoinsTailnet = (
  card: BackupCard | null,
  sameDrone: boolean | null,
): boolean => sameDrone === true && card?.tailnetNode === true;

export interface BackupMarks {
  remote: boolean;
  encryption: boolean;
  fight: boolean;
  ssh: boolean;
}

export const NO_MARKS: BackupMarks = {
  remote: false,
  encryption: false,
  fight: false,
  ssh: false,
};

export function backupMarks(
  card: BackupCard | null,
  now: {
    rejoins: boolean;
    selfHosted: boolean;
    controlServer: string;
    encryption: boolean;
    fightMode: boolean;
    sshMode: SshMode;
  },
): BackupMarks {
  if (card === null) return NO_MARKS;
  return {
    remote:
      now.rejoins ||
      (card.controlServer !== "" &&
        now.selfHosted &&
        now.controlServer.trim() === card.controlServer),
    encryption: (card.encryption !== null) === now.encryption,
    fight: card.fightMode === now.fightMode,
    ssh: card.passwordLogin && now.sshMode === "password",
  };
}

export const onlineNow = (hostname: string, peers: TailnetPeer[]): boolean =>
  peers.some(
    (peer) =>
      peer.online && peer.hostName.toLowerCase() === hostname.toLowerCase(),
  );
