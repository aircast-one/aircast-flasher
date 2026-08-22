import { Check } from "lucide-react";

import { cn } from "@/lib/utils";
import { STEP, type WizardStep } from "@/components/wizard-types";

export interface SidebarStep {
  id: WizardStep;
  label: string;
}

export const WIZARD_STEPS: SidebarStep[] = [
  { id: STEP.os, label: "Operating system" },
  { id: STEP.network, label: "Network & access" },
  { id: STEP.storage, label: "Storage" },
  { id: STEP.write, label: "Write" },
];

export function WizardSidebar({
  current,
  highestReached,
  writing,
  onSelect,
  telemetry,
  onTelemetry,
}: {
  current: WizardStep;
  highestReached: WizardStep;
  writing: boolean;
  onSelect: (step: WizardStep) => void;
  telemetry: boolean | null;
  onTelemetry: (share: boolean) => void;
}) {
  return (
    <aside className="flex w-[220px] shrink-0 flex-col border-r border-sidebar-border bg-sidebar">
      <div className="flex items-center gap-2.5 px-5 py-5">
        <img
          src="/aircast-logo.png"
          alt=""
          aria-hidden="true"
          className="size-6 rounded-[5px]"
        />
        <span className="text-sm font-semibold text-sidebar-foreground">
          Aircast Flasher
        </span>
      </div>

      <nav className="flex flex-col gap-1 px-3" aria-label="Setup steps">
        <p className="px-2 pb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Setup steps
        </p>
        {WIZARD_STEPS.map((s, i) => {
          const isActive = s.id === current;
          const isDone = s.id < current;
          const isClickable = s.id < current && !writing;
          return (
            <button
              key={s.id}
              type="button"
              disabled={!isClickable}
              onClick={() => isClickable && onSelect(s.id)}
              aria-current={isActive ? "step" : undefined}
              className={cn(
                "flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors outline-none",
                "focus-visible:ring-2 focus-visible:ring-ring/50",
                isActive &&
                  "bg-primary font-medium text-primary-foreground shadow-sm",
                !isActive &&
                  isDone &&
                  "text-sidebar-foreground hover:bg-sidebar-accent",
                !isActive && !isDone && "text-muted-foreground/60",
                isClickable ? "cursor-pointer" : "cursor-default",
              )}
            >
              <span
                className={cn(
                  "flex size-5 shrink-0 items-center justify-center rounded-full text-[0.7rem] font-semibold",
                  isActive &&
                    "bg-primary-foreground/20 text-primary-foreground",
                  !isActive && isDone && "bg-primary/20 text-primary",
                  !isActive &&
                    !isDone &&
                    "border border-border text-muted-foreground/60",
                )}
              >
                {isDone ? <Check className="size-3" /> : i + 1}
              </span>
              <span className="truncate">{s.label}</span>
            </button>
          );
        })}
      </nav>

      <div className="mt-auto flex flex-col gap-1.5 px-5 py-4 text-xs text-muted-foreground/60">
        <span>
          {writing
            ? "Writing in progress"
            : highestReached >= STEP.write
              ? "Ready to write"
              : "Configure your device"}
        </span>
        <button
          type="button"
          onClick={() => onTelemetry(!telemetry)}
          className="self-start rounded-sm text-left underline-offset-2 outline-none hover:text-sidebar-foreground hover:underline focus-visible:ring-2 focus-visible:ring-ring/50"
        >
          Diagnostics {telemetry ? "on" : "off"}
        </button>
      </div>
    </aside>
  );
}
