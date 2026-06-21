import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export function SelectableCard({
  icon,
  title,
  selected,
  disabled,
  onClick,
  children,
}: {
  icon: ReactNode;
  title: ReactNode;
  selected: boolean;
  disabled?: boolean;
  onClick: () => void;
  children?: ReactNode;
}) {
  return (
    <button
      type="button"
      disabled={disabled}
      onClick={onClick}
      aria-pressed={selected}
      className={cn(
        "group flex w-full items-start gap-4 rounded-xl border bg-card p-4 text-left transition-all outline-none",
        "focus-visible:ring-3 focus-visible:ring-ring/50",
        "disabled:pointer-events-none disabled:opacity-50",
        selected
          ? "border-primary bg-primary/10 ring-1 ring-primary"
          : "border-border hover:border-primary/40 hover:bg-muted/40",
      )}
    >
      <span
        className={cn(
          "mt-0.5 flex size-10 shrink-0 items-center justify-center rounded-lg transition-colors [&_svg]:size-5",
          selected
            ? "bg-primary/20 text-primary"
            : "bg-muted text-muted-foreground group-hover:text-foreground",
        )}
      >
        {icon}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-medium text-foreground">{title}</span>
        {children}
      </span>
    </button>
  );
}
