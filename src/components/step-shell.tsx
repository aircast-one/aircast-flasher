import type { ReactNode } from "react";

import { Button } from "@/components/ui/button";

export function StepShell({
  heading,
  description,
  headerAction,
  children,
  back,
  next,
}: {
  heading: string;
  description?: string;
  headerAction?: ReactNode;
  children: ReactNode;
  back?: { onClick: () => void; disabled?: boolean } | null;
  next?: {
    label: string;
    onClick: () => void;
    disabled?: boolean;
  } | null;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="min-h-0 flex-1 overflow-y-auto px-8 pb-4">
        <div className="flex items-start justify-between gap-4 pt-7 pb-4">
          <div className="flex flex-col gap-1">
            <h1 className="text-2xl font-bold tracking-tight">{heading}</h1>
            {description ? (
              <p className="text-sm text-muted-foreground">{description}</p>
            ) : null}
          </div>
          {headerAction ? <div className="shrink-0">{headerAction}</div> : null}
        </div>
        {children}
      </div>

      {back || next ? (
        <div className="flex items-center justify-between gap-3 border-t border-border bg-background/60 px-8 py-4">
          <div>
            {back ? (
              <Button
                type="button"
                variant="secondary"
                size="lg"
                onClick={back.onClick}
                disabled={back.disabled}
              >
                Back
              </Button>
            ) : null}
          </div>
          <div>
            {next ? (
              <Button
                type="button"
                size="lg"
                className="min-w-32"
                onClick={next.onClick}
                disabled={next.disabled}
              >
                {next.label}
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}
    </div>
  );
}
