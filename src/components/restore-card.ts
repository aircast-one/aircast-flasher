import type {
  Backup,
  BackupCard,
  Settings,
  SshMode,
  TailnetPeer,
} from "@/types";
import type { FieldProblem } from "@/components/step-network.validation";
import type { RestoreDraft } from "@/components/wizard-types";

export interface DroneBackups {
  hostname: string;
  newest: Backup;
  earlier: Backup[];
}

export const droneBackups = (backups: Backup[]): DroneBackups[] =>
  [...new Set(backups.map((b) => b.hostname))]
    .sort((a, b) => a.localeCompare(b))
    .map((hostname) => {
      const [newest, ...earlier] = backups.filter(
        (b) => b.hostname === hostname,
      );
      return { hostname, newest, earlier };
    });

export function restoreProblems(
  restore: RestoreDraft | null,
  backups: Backup[],
): FieldProblem[] {
  if (restore === null) return [];
  if (restore.backup === null)
    return [
      {
        field: "restore-drone",
        message: "Pick the drone whose backup goes on this card",
      },
    ];
  if (!backups.some((b) => b.id === restore.backup))
    return [
      {
        field: "restore-drone",
        message:
          "Pick the backup to restore — the chosen one is no longer on this computer",
      },
    ];
  return restore.sameDrone === null
    ? [
        {
          field: "restore-same-drone",
          message:
            "Say whether the backup goes back onto the same drone or a new one",
        },
      ]
    : [];
}

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
