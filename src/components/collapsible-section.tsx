import { useState, type ReactNode } from "react";
import { ChevronDown } from "lucide-react";

export function CollapsibleSection({
  title,
  forceOpen,
  initiallyOpen,
  children,
}: {
  title: string;
  forceOpen: boolean;
  initiallyOpen: boolean;
  children: ReactNode;
}) {
  const [open, setOpen] = useState(initiallyOpen);
  const expanded = open || forceOpen;

  return (
    <div className="flex flex-col gap-3 border-t border-border pt-5">
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={expanded}
        className="flex items-center gap-1.5 text-sm font-medium text-foreground"
      >
        {title}
        <ChevronDown
          className={`size-4 text-muted-foreground transition-transform ${
            expanded ? "rotate-180" : ""
          }`}
        />
      </button>
      {expanded ? children : null}
    </div>
  );
}
