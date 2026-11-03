import type { ReactNode } from "react";

export function FromBackup() {
  return (
    <span className="shrink-0 rounded-full bg-primary/10 px-1.5 py-0.5 text-xs font-medium text-primary">
      from backup
    </span>
  );
}

export function SettingCard({
  icon,
  title,
  description,
  action,
  fromBackup = false,
  children,
}: {
  icon: ReactNode;
  title: string;
  description: ReactNode;
  action: ReactNode;
  fromBackup?: boolean;
  children?: ReactNode;
}) {
  return (
    <section className="flex flex-col rounded-xl border border-border bg-card">
      <div className="flex items-center gap-3 p-4">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-lg bg-muted text-muted-foreground [&_svg]:size-4.5">
          {icon}
        </span>
        <div className="flex min-w-0 flex-1 flex-col">
          <h2 className="flex items-center gap-2 text-sm font-medium">
            {title}
            {fromBackup ? <FromBackup /> : null}
          </h2>
          <p className="text-sm text-muted-foreground">{description}</p>
        </div>
        {action}
      </div>
      {children ? (
        <div className="flex flex-col gap-4 border-t border-border p-4">
          {children}
        </div>
      ) : null}
    </section>
  );
}
