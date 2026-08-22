import { BarChart3 } from "lucide-react";

import { Button } from "@/components/ui/button";

export function ConsentBanner({
  asked,
  onAnswer,
}: {
  asked: boolean;
  onAnswer: (share: boolean) => void;
}) {
  if (asked) return null;

  return (
    <div
      className="flex items-center justify-center gap-3 border-b border-border bg-muted/40 px-4 py-2 text-sm"
      data-testid="consent-banner"
    >
      <BarChart3 className="size-4 shrink-0 text-muted-foreground" />
      <span className="text-muted-foreground">
        Share anonymous diagnostics — which step failed, how fast the card wrote.
        Never your network name, passphrase, keys or hostname.
      </span>
      <Button size="sm" variant="secondary" onClick={() => onAnswer(true)}>
        Share
      </Button>
      <Button size="sm" variant="ghost" onClick={() => onAnswer(false)}>
        No thanks
      </Button>
    </div>
  );
}
