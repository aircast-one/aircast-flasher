import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

const photos = import.meta.glob<string>("../assets/boards/*.webp", {
  eager: true,
  import: "default",
});

export function boardPhoto(id: string): string | undefined {
  return photos[`../assets/boards/${id}.webp`];
}

export function BoardCard({
  photo,
  title,
  selected,
  onClick,
  board,
  compact,
  children,
}: {
  photo: string | undefined;
  title: ReactNode;
  selected: boolean;
  onClick: () => void;
  board?: string;
  compact?: boolean;
  children?: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      data-board={board}
      className={cn(
        "group flex w-full gap-3 rounded-xl border bg-card p-3 text-left transition-all outline-none",
        "focus-visible:ring-3 focus-visible:ring-ring/50",
        compact ? "items-center" : "flex-col",
        selected
          ? "border-primary bg-primary/10 ring-1 ring-primary"
          : "border-border hover:border-primary/40 hover:bg-muted/40",
      )}
    >
      <span
        className={cn(
          "flex shrink-0 items-center justify-center overflow-hidden rounded-lg bg-white",
          compact ? "h-14 w-20" : "aspect-[8/5] w-full",
        )}
      >
        {photo ? (
          <img
            src={photo}
            alt=""
            draggable={false}
            className="size-full object-contain p-1.5"
          />
        ) : null}
      </span>
      <span className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="font-medium text-foreground">{title}</span>
        {children}
      </span>
    </button>
  );
}
