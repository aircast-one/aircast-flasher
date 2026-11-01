import { ArchiveRestore, CopyPlus, RotateCcw } from "lucide-react";

import { SettingCard } from "@/components/setting-card";
import { SelectableCard } from "@/components/selectable-card";
import { Switch } from "@/components/ui/switch";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { backupLabel } from "@/lib/format";
import type { RestoreDraft } from "@/components/wizard-types";
import type { Backup } from "@/types";

export function restoreProblems(
  restore: RestoreDraft | null,
  backups: Backup[],
): string[] {
  if (restore === null) return [];
  if (!backups.some((b) => b.id === restore.backup))
    return [
      "Pick the backup to restore — the chosen one is no longer on this computer",
    ];
  return restore.sameDrone === null
    ? ["Say whether the backup goes back onto the same drone or a new one"]
    : [];
}

export function RestoreSection({
  backups,
  restore,
  onRestore,
}: {
  backups: Backup[];
  restore: RestoreDraft | null;
  onRestore: (next: RestoreDraft | null) => void;
}) {
  const labelOf = (id: string) => {
    const backup = backups.find((b) => b.id === id);
    return backup ? backupLabel(backup) : "Pick a backup";
  };

  return (
    <SettingCard
      icon={<ArchiveRestore />}
      title="Restore settings from a backup"
      description={
        backups.length === 0
          ? "Back up a drone from Devices first, then its settings can go onto this card."
          : "Puts a saved drone's settings on the card. It restores them on first boot and restarts once."
      }
      action={
        <Switch
          aria-label="Restore settings"
          checked={restore !== null}
          disabled={backups.length === 0 && restore === null}
          onCheckedChange={(on) =>
            onRestore(on ? { backup: backups[0].id, sameDrone: null } : null)
          }
        />
      }
    >
      {restore ? (
        <>
          <Select
            value={restore.backup}
            onValueChange={(value) =>
              onRestore({ ...restore, backup: String(value) })
            }
          >
            <SelectTrigger aria-label="Backup to restore" className="w-full">
              <SelectValue>{(value) => labelOf(String(value))}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {backups.map((backup) => (
                <SelectItem key={backup.id} value={backup.id}>
                  {backupLabel(backup)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>

          <div
            role="group"
            aria-label="Which drone this card is for"
            className="grid grid-cols-2 gap-3"
          >
            <SelectableCard
              icon={<RotateCcw />}
              title="Same drone"
              selected={restore.sameDrone === true}
              onClick={() => onRestore({ ...restore, sameDrone: true })}
            >
              <span className="text-sm text-muted-foreground">
                Reflashing this aircraft. It comes back as the same drone, under
                the backup's name.
              </span>
            </SelectableCard>
            <SelectableCard
              icon={<CopyPlus />}
              title="New drone"
              selected={restore.sameDrone === false}
              onClick={() => onRestore({ ...restore, sameDrone: false })}
            >
              <span className="text-sm text-muted-foreground">
                Copy the setup onto another aircraft. It gets its own identity
                and cloud link.
              </span>
            </SelectableCard>
          </div>
        </>
      ) : null}
    </SettingCard>
  );
}
