import { AlertTriangle, HardDrive, RefreshCw } from "lucide-react";

import { Alert, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";
import { SelectableCard } from "@/components/selectable-card";
import { StepShell } from "@/components/step-shell";
import { cn } from "@/lib/utils";
import type { BlockDevice } from "@/types";

export function StepStorage({
  devices,
  devicesLoading,
  selectedDisk,
  onSelectDisk,
  onRefresh,
  canProceed,
  onNext,
}: {
  devices: BlockDevice[];
  devicesLoading: boolean;
  selectedDisk: string;
  onSelectDisk: (path: string) => void;
  onRefresh: () => void;
  canProceed: boolean;
  onNext: () => void;
}) {
  return (
    <StepShell
      heading="Select your storage device"
      description="Choose the SD card or drive you want to write the image to."
      headerAction={
        <Button
          type="button"
          variant="secondary"
          size="sm"
          onClick={onRefresh}
          disabled={devicesLoading}
        >
          <RefreshCw className={cn(devicesLoading && "animate-spin")} />
          {devicesLoading ? "Refreshing…" : "Refresh"}
        </Button>
      }
      next={{ label: "Next", onClick: onNext, disabled: !canProceed }}
    >
      <div className="flex flex-col gap-4">
        {devices.length === 0 ? (
          <div className="flex flex-col items-center gap-2 rounded-xl border border-dashed border-border py-14 text-center">
            <HardDrive className="size-8 text-muted-foreground/60" />
            <p className="text-sm text-muted-foreground">
              Insert an SD card and click Refresh.
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
