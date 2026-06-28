import { AlertTriangle, HardDrive } from "lucide-react";

import { Alert, AlertTitle } from "@/components/ui/alert";
import { SelectableCard } from "@/components/selectable-card";
import { StepShell } from "@/components/step-shell";
import type { BlockDevice } from "@/types";

export function StepStorage({
  devices,
  devicesLoading,
  selectedDisk,
  onSelectDisk,
  canProceed,
  onBack,
  onFlash,
}: {
  devices: BlockDevice[];
  devicesLoading: boolean;
  selectedDisk: string;
  onSelectDisk: (path: string) => void;
  canProceed: boolean;
  onBack: () => void;
  onFlash: () => void;
}) {
  return (
    <StepShell
      heading="Select your storage device"
      description="Insert your SD card now — it'll be detected automatically — then write the image."
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

        {selectedDisk !== "" ? (
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
