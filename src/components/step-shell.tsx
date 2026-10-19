import { useEffect, useRef, useState, type ReactNode } from "react";
import { CircleAlert } from "lucide-react";

import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Button } from "@/components/ui/button";

export interface Problem {
  message: string;
  fix?: () => void;
}

export const focusField = (id: string) => () =>
  requestAnimationFrame(() => {
    const field = document.getElementById(id);
    field?.scrollIntoView({ block: "center", behavior: "smooth" });
    field?.focus({ preventScroll: true });
  });

function ProblemList({
  problems,
  attempt,
}: {
  problems: Problem[];
  attempt: number;
}) {
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => ref.current?.focus(), [attempt]);

  return (
    <Alert
      ref={ref}
      tabIndex={-1}
      variant="destructive"
      aria-labelledby="step-problems-title"
      data-testid="step-problems"
      className="outline-none"
    >
      <CircleAlert />
      <AlertTitle id="step-problems-title">
        {problems.length === 1
          ? "Fix this to continue"
          : `Fix these ${problems.length} things to continue`}
      </AlertTitle>
      <AlertDescription>
        <ul className="flex flex-col gap-1">
          {problems.map(({ message, fix }) => (
            <li key={message}>
              {fix ? (
                <button
                  type="button"
                  onClick={fix}
                  className="text-left underline underline-offset-3 hover:text-foreground"
                >
                  {message}
                </button>
              ) : (
                message
              )}
            </li>
          ))}
        </ul>
      </AlertDescription>
    </Alert>
  );
}

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
    problems?: Problem[];
  } | null;
}) {
  const [attempt, setAttempt] = useState(0);
  const problems = next?.problems ?? [];
  const showProblems = attempt > 0 && problems.length > 0;

  const submit = (onClick: () => void) =>
    problems.length > 0 ? setAttempt((n) => n + 1) : onClick();

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
        <div className="flex flex-col gap-3 border-t border-border bg-background/60 px-8 py-4">
          {showProblems ? (
            <ProblemList problems={problems} attempt={attempt} />
          ) : null}
          <div className="flex items-center justify-between gap-3">
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
                  onClick={() => submit(next.onClick)}
                >
                  {next.label}
                </Button>
              ) : null}
            </div>
          </div>
        </div>
      ) : null}
    </div>
  );
}
