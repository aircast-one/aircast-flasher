import { useEffect, useState } from "react";
import { Check, Copy, ExternalLink, Loader2, Wifi } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { joinWifi, probeDevice, track } from "@/api";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/format";

const PROBE_INTERVAL_MS = 3000;
const HOTSPOT_URL = "http://10.42.0.1";
const HOTSPOT_PASSWORD = "raspberry";

type JoinState =
  | { phase: "idle" }
  | { phase: "joining" }
  | { phase: "failed"; error: string };

const COPIED_FEEDBACK_MS = 2000;

function CopyableAddress({ address }: { address: string }) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  return (
    <button
      type="button"
      className="flex items-center gap-2 self-start font-mono text-sm break-all hover:text-primary"
      onClick={() => {
        navigator.clipboard
          ?.writeText(address)
          .then(() => setCopied(true))
          .catch(() => setCopied(false));
      }}
      aria-label={`Copy ${address}`}
    >
      {address}
      {copied ? (
        <Check className="size-4 shrink-0 text-green-500 dark:text-green-400" />
      ) : (
        <Copy className="size-4 shrink-0 text-muted-foreground" />
      )}
    </button>
  );
}

export function DeviceFinder({
  hostname,
  remoteEnrolled,
}: {
  hostname: string;
  remoteEnrolled: boolean;
}) {
  const [foundUrl, setFoundUrl] = useState<string | null>(null);
  const [join, setJoin] = useState<JoinState>({ phase: "idle" });
  const lanUrl = `http://${hostname}.local`;

  useEffect(() => {
    if (foundUrl !== null) return;
    let cancelled = false;
    const probe = () =>
      void Promise.all(
        [lanUrl, HOTSPOT_URL].map((url) =>
          probeDevice(url).then(
            (ok) => (ok ? url : null),
            () => null,
          ),
        ),
      ).then((results) => {
        const hit = results.find((r) => r !== null) ?? null;
        if (hit !== null && !cancelled) {
          setFoundUrl(hit);
          track("device_found", {
            via: hit === HOTSPOT_URL ? "hotspot" : "lan",
          });
        }
      });
    probe();
    const timer = setInterval(probe, PROBE_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [foundUrl, lanUrl]);

  function handleJoinHotspot() {
    setJoin({ phase: "joining" });
    track("hotspot_join_attempt");
    joinWifi(hostname, HOTSPOT_PASSWORD)
      .then(() => setJoin({ phase: "idle" }))
      .catch((err) => {
        const error = errorMessage(err, "Could not join the hotspot.");
        track("hotspot_join_failed", { error });
        setJoin({ phase: "failed", error });
      });
  }

  if (foundUrl !== null) {
    return (
      <div className="flex w-full flex-col gap-3 rounded-xl border border-green-500/30 bg-green-500/5 px-4 py-3 text-left">
        <div className="flex flex-col gap-0.5">
          <span className="text-xs font-medium tracking-wide text-green-600 uppercase dark:text-green-400">
            Device online
          </span>
          <CopyableAddress address={foundUrl} />
          {foundUrl === HOTSPOT_URL ? (
            <span className="text-xs text-muted-foreground">
              You're connected over the device's hotspot.
            </span>
          ) : null}
        </div>
        <Button
          type="button"
          size="sm"
          className="self-start"
          onClick={() => void openUrl(foundUrl)}
        >
          <ExternalLink className="size-4" />
          Open device page
        </Button>
      </div>
    );
  }

  return (
    <div className="flex w-full flex-col gap-3 rounded-xl border border-border px-4 py-3 text-left">
      <div className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="size-4 shrink-0 animate-spin" />
        <span>Waiting for the device. Checking these addresses:</span>
      </div>
      <CopyableAddress address={`${hostname}.local`} />
      <div className="flex flex-col gap-1 text-xs text-muted-foreground">
        <span>
          If your WiFi isn't found, the device starts a hotspot named{" "}
          <span className="font-mono">{hostname}</span> (password{" "}
          <span className="font-mono">{HOTSPOT_PASSWORD}</span>).
        </span>
        {remoteEnrolled ? (
          <span>It also joins your private network on first boot.</span>
        ) : null}
      </div>
      <div className="flex items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="secondary"
          className="self-start"
          disabled={join.phase === "joining"}
          onClick={handleJoinHotspot}
        >
          {join.phase === "joining" ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <Wifi className="size-4" />
          )}
          Join its hotspot
        </Button>
        {join.phase === "failed" ? (
          <span className="text-xs text-destructive">{join.error}</span>
        ) : null}
      </div>
    </div>
  );
}
