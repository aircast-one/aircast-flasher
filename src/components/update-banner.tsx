import { useEffect, useState } from "react";
import type { Update } from "@tauri-apps/plugin-updater";
import { AlertTriangle, Download, Loader2 } from "lucide-react";
import { checkForUpdate, installUpdate } from "@/api";
import { Button } from "@/components/ui/button";

export function UpdateBanner({ suspended }: { suspended: boolean }) {
  const [update, setUpdate] = useState<Update | null>(null);
  const [installing, setInstalling] = useState(false);
  const [dismissed, setDismissed] = useState(false);
  const [downloaded, setDownloaded] = useState(0);
  const [total, setTotal] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let active = true;
    checkForUpdate().then((u) => {
      if (active) setUpdate(u);
    });
    return () => {
      active = false;
    };
  }, []);

  if (!update || dismissed || (suspended && !installing)) return null;

  async function handleUpdate() {
    if (!update) return;
    setError(null);
    setDownloaded(0);
    setTotal(null);
    setInstalling(true);
    try {
      await installUpdate(update, (event) => {
        if (event.event === "Started") setTotal(event.data.contentLength ?? null);
        if (event.event === "Progress")
          setDownloaded((bytes) => bytes + event.data.chunkLength);
      });
    } catch (cause) {
      setError(String(cause));
      setInstalling(false);
    }
  }

  const percent =
    total !== null && total > 0
      ? Math.min(100, Math.round((downloaded / total) * 100))
      : null;

  return (
    <div className="flex items-center justify-center gap-3 border-b border-border bg-muted/40 px-4 py-2 text-sm text-foreground">
      {error ? (
        <span className="flex items-center gap-2 text-destructive">
          <AlertTriangle className="size-4 shrink-0" />
          Update to v{update.version} failed: {error}
        </span>
      ) : (
        <span className="text-muted-foreground">
          Update available{" "}
          <span className="font-medium text-foreground">v{update.version}</span>
        </span>
      )}
      <Button
        size="sm"
        variant="secondary"
        onClick={handleUpdate}
        disabled={installing}
      >
        {installing ? <Loader2 className="animate-spin" /> : <Download />}
        {installing
          ? percent === null
            ? "Downloading…"
            : `Downloading ${percent}%`
          : error
            ? "Try again"
            : "Update & restart"}
      </Button>
      {!installing && (
        <Button size="sm" variant="ghost" onClick={() => setDismissed(true)}>
          Later
        </Button>
      )}
    </div>
  );
}
