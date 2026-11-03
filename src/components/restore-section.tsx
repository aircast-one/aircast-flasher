import { useState } from "react";
import {
  ArchiveRestore,
  ChevronRight,
  CopyPlus,
  RotateCcw,
  TriangleAlert,
} from "lucide-react";

import { SettingCard } from "@/components/setting-card";
import { SelectableCard } from "@/components/selectable-card";
import { Switch } from "@/components/ui/switch";
import { RadioGroup, RadioGroupItem } from "@/components/ui/radio-group";
import { formatDateTime, timeAgo } from "@/lib/format";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import { droneBackups, onlineNow } from "@/components/restore-card";
import type { RestoreDraft } from "@/components/wizard-types";
import type { Backup } from "@/types";

const OPTION =
  "flex cursor-pointer items-center gap-3 rounded-lg border border-border px-3 py-2.5 transition-colors hover:bg-muted/40 has-data-checked:border-primary has-data-checked:bg-primary/5";

export function RestoreSection({
  backups,
  restore,
  onRestore,
}: {
  backups: Backup[];
  restore: RestoreDraft | null;
  onRestore: (next: RestoreDraft | null) => void;
}) {
  const peers = useLocalTailscale()?.peers ?? [];
  const [versionsOpen, setVersionsOpen] = useState(false);
  const drones = droneBackups(backups);
  const chosen = backups.find((b) => b.id === restore?.backup) ?? null;
  const drone = drones.find((d) => d.hostname === chosen?.hostname) ?? null;
  const online = chosen !== null && onlineNow(chosen.hostname, peers);
  const now = Date.now();

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
            onRestore(on ? { backup: null, sameDrone: null } : null)
          }
        />
      }
    >
      {restore ? (
        <>
          <div className="flex flex-col gap-2">
            <p id="restore-drone-label" className="text-sm font-medium">
              Which drone's settings?
            </p>
            <RadioGroup
              aria-labelledby="restore-drone-label"
              value={drone?.hostname ?? null}
              onValueChange={(hostname) => {
                const picked = drones.find((d) => d.hostname === hostname);
                if (picked)
                  onRestore({ backup: picked.newest.id, sameDrone: null });
              }}
            >
              {drones.map((d, i) => (
                <label key={d.hostname} className={OPTION}>
                  <RadioGroupItem
                    value={d.hostname}
                    id={i === 0 ? "restore-drone" : undefined}
                  />
                  <span className="min-w-0 flex-1 truncate text-sm font-medium">
                    {d.hostname}
                  </span>
                  <span className="shrink-0 text-xs text-muted-foreground">
                    Backed up {timeAgo(d.newest.created, now)}
                  </span>
                </label>
              ))}
            </RadioGroup>
          </div>

          {chosen !== null && drone !== null ? (
            <div className="flex flex-col gap-2">
              <p id="restore-chosen" className="text-sm text-muted-foreground">
                {chosen.id === drone.newest.id
                  ? `Uses ${drone.hostname}'s newest backup, from ${formatDateTime(chosen.created)}.`
                  : `Uses an earlier backup of ${drone.hostname}, from ${formatDateTime(chosen.created)}.`}
              </p>
              {drone.earlier.length > 0 ? (
                <details
                  id="restore-versions"
                  open={versionsOpen || chosen.id !== drone.newest.id}
                  onToggle={(e) => setVersionsOpen(e.currentTarget.open)}
                  className="group"
                >
                  <summary className="flex cursor-pointer list-none items-center gap-2 text-sm font-medium text-muted-foreground hover:text-foreground [&::-webkit-details-marker]:hidden">
                    <ChevronRight className="size-4 transition-transform group-open:rotate-90" />
                    Earlier backups ({drone.earlier.length})
                  </summary>
                  <RadioGroup
                    aria-label="Backup of this drone"
                    className="mt-2"
                    value={chosen.id}
                    onValueChange={(id) =>
                      onRestore({ ...restore, backup: String(id) })
                    }
                  >
                    {[drone.newest, ...drone.earlier].map((b) => (
                      <label key={b.id} className={OPTION}>
                        <RadioGroupItem value={b.id} />
                        <span className="min-w-0 flex-1 truncate text-sm">
                          {formatDateTime(b.created)}
                          {b.id === drone.newest.id ? " (newest)" : ""}
                        </span>
                        <span className="shrink-0 text-xs text-muted-foreground">
                          {timeAgo(b.created, now)}
                        </span>
                      </label>
                    ))}
                  </RadioGroup>
                </details>
              ) : null}
            </div>
          ) : null}

          {chosen !== null && chosen.card === null ? (
            <p id="restore-no-card" className="text-sm text-muted-foreground">
              This backup doesn't record fight mode, encryption or remote
              access — check Access yourself.
            </p>
          ) : null}

          {chosen !== null ? (
            <div
              role="group"
              aria-label="Which drone this card is for"
              className="grid grid-cols-2 gap-3"
            >
              <SelectableCard
                id="restore-same-drone"
                icon={<RotateCcw />}
                title="Same drone"
                selected={restore.sameDrone === true}
                onClick={() => onRestore({ ...restore, sameDrone: true })}
              >
                <span className="text-sm text-muted-foreground">
                  Reflashing this aircraft. It comes back as the same drone,
                  under the backup's name.
                </span>
                {online ? (
                  <span
                    id="restore-online"
                    className="mt-1 flex items-start gap-1.5 text-sm text-amber-600 dark:text-amber-400"
                  >
                    <TriangleAlert className="mt-0.5 size-4 shrink-0" />
                    {chosen.hostname} is online right now. Only choose this if
                    you're replacing its card.
                  </span>
                ) : null}
              </SelectableCard>
              <SelectableCard
                icon={<CopyPlus />}
                title="New drone"
                selected={restore.sameDrone === false}
                onClick={() => onRestore({ ...restore, sameDrone: false })}
              >
                <span className="text-sm text-muted-foreground">
                  Copy the setup onto another aircraft. It gets its own
                  identity and cloud link.
                </span>
              </SelectableCard>
            </div>
          ) : null}
        </>
      ) : null}
    </SettingCard>
  );
}
