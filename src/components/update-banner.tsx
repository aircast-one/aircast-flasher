import { useEffect, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";
import { Download, Loader2 } from "lucide-react";
import { checkForUpdate, installUpdate } from "@/api";
import { Button } from "@/components/ui/button";

// Unobtrusive top-of-window banner that appears only when the updater reports a
// newer release. It checks once on mount; any failure (404 endpoint, dev build)
// is swallowed in `checkForUpdate`, so the banner simply never shows. It never
// blocks the wizard underneath.
export function UpdateBanner() {
  const [update, setUpdate] = useState<Update | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  useEffect(() => {
    let active = true;
    checkForUpdate().then((u) => {
      if (active) setUpdate(u);
    });
    return () => {
      active = false;
    };
  }, []);

  if (!update || dismissed) return null;

  async function handleUpdate() {
    if (!update) return;
    setInstalling(true);
    try {
      await installUpdate(update);
      // installUpdate relaunches; we only reach here if it didn't.
    } catch {
      setInstalling(false);
    }
  }

  return (
    <div className="flex items-center justify-center gap-3 border-b border-border bg-muted/40 px-4 py-2 text-sm text-foreground">
      <span className="text-muted-foreground">
        Update available{" "}
        <span className="font-medium text-foreground">v{update.version}</span>
      </span>
      <Button size="sm" onClick={handleUpdate} disabled={installing}>
        {installing ? (
          <Loader2 className="animate-spin" />
        ) : (
          <Download />
        )}
        {installing ? "Updating…" : "Update & restart"}
      </Button>
      {!installing && (
        <Button
          size="sm"
          variant="ghost"
          onClick={() => setDismissed(true)}
        >
          Later
        </Button>
      )}
    </div>
  );
}
