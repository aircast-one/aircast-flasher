import { AlertTriangle, HardDrive } from "lucide-react";

import { Alert, AlertTitle } from "@/components/ui/alert";
import { SelectableCard } from "@/components/selectable-card";
import { StepShell } from "@/components/step-shell";
import type { BlockDevice } from "@/types";

export interface SummaryItem {
  label: string;
  value: string;
}

export function StepStorage({
  devices,
  devicesLoading,
  selectedDisk,
  onSelectDisk,
  summary,
  canProceed,
  onBack,
  onFlash,
}: {
  devices: BlockDevice[];
  devicesLoading: boolean;
  selectedDisk: string;
  onSelectDisk: (path: string) => void;
  summary: SummaryItem[];
  canProceed: boolean;
  onBack: () => void;
  onFlash: () => void;
}) {
  const selected = devices.find((d) => d.path === selectedDisk) ?? null;

  return (
    <StepShell
      heading="Storage"
      description="Insert your SD card now — it'll be detected automatically — then check the settings below and write."
      back={{ onClick: onBack }}
      next={{ label: "Flash SD Card", onClick: onFlash, disabled: !canProceed }}
    >
      <div className="flex flex-col gap-4">
        {devices.length === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border py-14 text-center">
            <HardDrive className="size-8 text-muted-foreground/60" />
            <p className="text-sm text-muted-foreground">
              {devicesLoading
                ? "Scanning for storage devices…"
                : "Insert an SD card — it'll be detected automatically."}
            </p>
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            {devices.map((d) => (
              <SelectableCard
                key={d.path}
                icon={<HardDrive />}
                title={d.name}
                selected={selectedDisk === d.path}
                onClick={() => onSelectDisk(d.path)}
              >
                <span className="text-sm text-muted-foreground">
                  {d.size_human}
                </span>
                {d.mounted && d.mount_points.length > 0 ? (
                  <span className="text-xs text-muted-foreground">
                    Mounted as {d.mount_points.join(", ")}
                  </span>
                ) : null}
              </SelectableCard>
            ))}
          </div>
        )}

        <div className="flex flex-col gap-3 rounded-xl border border-border p-4">
          <h2 className="text-sm font-medium">What will be written</h2>
          <dl className="flex flex-col gap-2">
            {[
              ...summary,
              {
                label: "Target card",
                value: selected
                  ? `${selected.name} · ${selected.size_human}`
                  : "No card selected yet",
              },
            ].map((item) => (
              <div
                key={item.label}
                className="flex items-baseline justify-between gap-4 text-sm"
              >
                <dt className="shrink-0 text-muted-foreground">{item.label}</dt>
                <dd className="text-right break-all">{item.value}</dd>
              </div>
            ))}
          </dl>
        </div>

        {selected ? (
          <Alert variant="destructive">
            <AlertTriangle />
            <AlertTitle>
              All data on the selected card will be erased.
            </AlertTitle>
          </Alert>
        ) : null}
      </div>
    </StepShell>
  );
}
