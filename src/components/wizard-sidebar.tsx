import { Check, Router } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { getVersion } from "@tauri-apps/api/app";

import { cn } from "@/lib/utils";
import { STEP, type WizardStep } from "@/components/wizard-types";

export interface SidebarStep {
  id: WizardStep;
  label: string;
}

export const WIZARD_STEPS: SidebarStep[] = [
  { id: STEP.os, label: "Operating system" },
  { id: STEP.network, label: "Network" },
  { id: STEP.access, label: "Access" },
  { id: STEP.storage, label: "Storage" },
  { id: STEP.write, label: "Write" },
];

function AppVersion() {
  const versionQuery = useQuery({
    queryKey: ["app-version"],
    queryFn: getVersion,
    staleTime: Infinity,
  });
  return versionQuery.data ? <span>v{versionQuery.data}</span> : null;
}

export function WizardSidebar({
  current,
  highestReached,
  writing,
  onSelect,
  onDevices,
  showingDevices,
  telemetry,
  onTelemetry,
}: {
  current: WizardStep;
  highestReached: WizardStep;
  writing: boolean;
  onSelect: (step: WizardStep) => void;
  onDevices: () => void;
  showingDevices: boolean;
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
          const isActive = !showingDevices && s.id === current;
          const isDone = s.id < current;
          const isClickable =
            !writing &&
            s.id <= highestReached &&
            !(s.id === current && !showingDevices);
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
                  (isDone || isClickable) &&
                  "text-sidebar-foreground hover:bg-sidebar-accent",
                !isActive &&
                  !isDone &&
                  !isClickable &&
                  "text-muted-foreground/60",
                isClickable ? "cursor-pointer" : "cursor-default",
              )}
            >
              <span
                className={cn(
                  "flex size-5 shrink-0 items-center justify-center rounded-full text-[0.7rem] font-semibold",
                  isActive &&
                    "bg-primary-foreground/20 text-primary-foreground",
                  !isActive &&
                    (isDone || isClickable) &&
                    "bg-primary/20 text-primary",
                  !isActive &&
                    !isDone &&
                    !isClickable &&
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

      <nav className="mt-4 flex flex-col gap-1 px-3" aria-label="Fleet">
        <p className="px-2 pb-1 text-xs font-medium tracking-wide text-muted-foreground uppercase">
          Fleet
        </p>
        <button
          type="button"
          disabled={writing}
          onClick={onDevices}
          aria-current={showingDevices ? "page" : undefined}
          className={cn(
            "flex items-center gap-3 rounded-lg px-3 py-2 text-sm transition-colors outline-none",
            "focus-visible:ring-2 focus-visible:ring-ring/50",
            writing && "cursor-default text-muted-foreground/60",
            !writing && "cursor-pointer",
            showingDevices &&
              !writing &&
              "bg-primary font-medium text-primary-foreground shadow-sm",
            !showingDevices &&
              !writing &&
              "text-sidebar-foreground hover:bg-sidebar-accent",
          )}
        >
          <span className="flex size-5 shrink-0 items-center justify-center">
            <Router className="size-4" />
          </span>
          <span className="truncate">Devices</span>
        </button>
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
        <AppVersion />
      </div>
    </aside>
  );
}
