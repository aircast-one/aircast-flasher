import { AlertTriangle, HardDrive } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { SelectableCard } from "@/components/selectable-card";
import { StepShell } from "@/components/step-shell";
import { cn } from "@/lib/utils";
import type { SummaryItem } from "@/components/wizard-summary";
import { STEP, type WizardStep } from "@/components/wizard-types";
import type { BlockDevice } from "@/types";

export type { SummaryItem };

export function StepStorage({
  devices,
  devicesLoading,
  selectedDisk,
  onSelectDisk,
  requiredCardBytes,
  summary,
  canProceed,
  onBack,
  onEditStep,
  onFlash,
}: {
  devices: BlockDevice[];
  devicesLoading: boolean;
  selectedDisk: string;
  onSelectDisk: (path: string) => void;
  requiredCardBytes: number | undefined;
  summary: SummaryItem[];
  canProceed: boolean;
  onBack: () => void;
  onEditStep: (step: WizardStep) => void;
  onFlash: () => void;
}) {
  const selected = devices.find((d) => d.path === selectedDisk) ?? null;
  const tooSmall = (d: BlockDevice) =>
    requiredCardBytes !== undefined && d.size > 0 && d.size < requiredCardBytes;

  const rows: SummaryItem[] = [
    ...summary,
    {
      label: "Target card",
      value: selected
        ? `${selected.name} · ${selected.size_human}`
        : "No card selected yet",
      step: STEP.storage,
    },
  ];

  return (
    <StepShell
      heading="Storage"
      description="Insert your SD card and it'll show up here on its own. Check the settings below, then write."
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
                : "Insert an SD card and it'll show up here."}
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
                disabled={tooSmall(d)}
                onClick={() => onSelectDisk(d.path)}
              >
                <span className="text-sm text-muted-foreground">
                  {d.size_human}
                </span>
                {tooSmall(d) ? (
                  <span className="text-xs text-destructive">
                    Too small for this image
                  </span>
                ) : null}
                {d.mounted && d.mount_points.length > 0 ? (
                  <span className="text-xs text-muted-foreground">
                    Mounted as {d.mount_points.join(", ")}
                  </span>
                ) : null}
              </SelectableCard>
            ))}
            {selected === null ? (
              <p className="text-sm text-muted-foreground">
                Pick the card to write to.
              </p>
            ) : null}
          </div>
        )}

        <div className="flex flex-col gap-3 rounded-xl border border-border p-4">
          <h2 className="text-sm font-medium">What will be written</h2>
          <dl className="flex flex-col gap-2">
            {rows.map(({ label, value, warn, step }) => (
              <div
                key={label}
                className="flex items-baseline justify-between gap-4 text-sm"
              >
                <dt className="shrink-0 text-muted-foreground">{label}</dt>
                <dd className="flex min-w-0 items-baseline gap-3">
                  <span
                    className={cn(
                      "text-right break-all",
                      warn && "font-medium text-destructive",
                    )}
                  >
                    {value}
                  </span>
                  {step === STEP.storage ? null : (
                    <button
                      type="button"
                      onClick={() => onEditStep(step)}
                      aria-label={`Change ${label}`}
                      className="shrink-0 rounded-sm text-muted-foreground underline underline-offset-2 outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring/50"
                    >
                      Change
                    </button>
                  )}
                </dd>
              </div>
            ))}
          </dl>
        </div>

        {selected ? (
          <Alert variant="destructive">
            <AlertTriangle />
            <AlertTitle>
              Erase {selected.name} · {selected.size_human}?
            </AlertTitle>
            <AlertDescription>
              Everything on {selected.path} is overwritten. This cannot be
              undone. You'll be asked for your password before the write starts
              — stay nearby.
            </AlertDescription>
          </Alert>
        ) : null}
      </div>
    </StepShell>
  );
}
