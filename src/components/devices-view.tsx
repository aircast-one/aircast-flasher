import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import {
  Check,
  Copy,
  ExternalLink,
  Loader2,
  RefreshCw,
  Share2,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { openDeviceWindow, probeDevices, track } from "@/api";
import { Button } from "@/components/ui/button";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import type { TailnetPeer } from "@/types";

const COPIED_FEEDBACK_MS = 2000;
const PROBE_STALE_MS = 30_000;
const HELP_URL = "https://aircast.one/docs/software/tailscale";

const deviceUrl = (peer: TailnetPeer) => `http://${peer.dnsName}`;

function CopyButton({ address }: { address: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <Button
      type="button"
      size="sm"
      variant="ghost"
      aria-label={`Copy ${address}`}
      onClick={() => {
        navigator.clipboard
          ?.writeText(address)
          .then(() => {
            setCopied(true);
            setTimeout(() => setCopied(false), COPIED_FEEDBACK_MS);
          })
          .catch(() => setCopied(false));
      }}
    >
      {copied ? (
        <Check className="size-4 text-green-500 dark:text-green-400" />
      ) : (
        <Copy className="size-4" />
      )}
    </Button>
  );
}

function DeviceRow({ peer }: { peer: TailnetPeer }) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-border px-4 py-3">
      <span
        className="size-2 shrink-0 rounded-full bg-green-500 dark:bg-green-400"
        aria-label="Online"
      />
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="flex items-center gap-2 truncate text-sm font-medium">
          {peer.hostName}
          {peer.sameTailnet ? null : (
            <Share2
              className="size-3.5 shrink-0 text-muted-foreground"
              aria-label="Shared from another tailnet"
            />
          )}
        </span>
        <span className="truncate font-mono text-xs text-muted-foreground">
          {peer.dnsName}
        </span>
      </div>
      <CopyButton address={peer.dnsName} />
      <Button
        type="button"
        size="sm"
        className="shrink-0"
        onClick={() => {
          track("device_open_from_list");
          void openDeviceWindow(deviceUrl(peer), peer.hostName);
        }}
      >
        <ExternalLink className="size-4" />
        Open
      </Button>
    </div>
  );
}

function Empty({ children }: { children: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-border px-4 py-6 text-center text-sm text-muted-foreground">
      {children}
    </div>
  );
}

function tailnetSummary(silent: number, offline: number): string | null {
  const parts = [
    silent > 0
      ? `${silent} online machine${silent === 1 ? "" : "s"} did not answer as an Aircast device`
      : null,
    offline > 0 ? `${offline} offline` : null,
  ].filter(Boolean);
  return parts.length === 0 ? null : `${parts.join(" · ")}.`;
}

function HelpButton({ label }: { label: string }) {
  return (
    <Button
      type="button"
      size="sm"
      variant="secondary"
      onClick={() => void openUrl(HELP_URL)}
    >
      <ExternalLink className="size-4" />
      {label}
    </Button>
  );
}

function NoDevices({
  summary,
  onFlash,
}: {
  summary: string | null;
  onFlash: () => void;
}) {
  return (
    <Empty>
      <p>No Aircast device is answering right now.</p>
      <p className="mt-1 text-xs">
        {summary ?? "Your tailnet has no other machines yet."}
      </p>
      <div className="mt-4 flex justify-center gap-2">
        <Button type="button" size="sm" onClick={onFlash}>
          Flash a card
        </Button>
        <HelpButton label="Why isn't my device here?" />
      </div>
    </Empty>
  );
}

export function DevicesView({ onFlash }: { onFlash: () => void }) {
  const local = useLocalTailscale();
  const queryClient = useQueryClient();

  const candidates = (local?.peers ?? []).filter(
    (peer) => peer.online && peer.os.toLowerCase() === "linux",
  );
  const addresses = candidates.map(deviceUrl);

  const probe = useQuery({
    queryKey: ["aircast-devices", addresses],
    queryFn: () => probeDevices(addresses),
    enabled: addresses.length > 0,
    staleTime: PROBE_STALE_MS,
  });

  const reachable = new Set(probe.data ?? []);
  const devices = candidates.filter((peer) => reachable.has(deviceUrl(peer)));
  const offline = (local?.peers ?? []).filter(
    (peer) => !peer.online && peer.os.toLowerCase() === "linux",
  ).length;
  const others = candidates.length - devices.length;

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-start justify-between gap-4 px-8 pt-7 pb-4">
        <div className="flex flex-col gap-1">
          <h1 className="text-2xl font-bold tracking-tight">Devices</h1>
          <p className="text-sm text-muted-foreground">
            Aircast devices answering on your tailnet, reachable from anywhere.
            Rechecked every few seconds.
          </p>
        </div>
        <Button
          type="button"
          variant="secondary"
          disabled={probe.isFetching}
          onClick={() => {
            void queryClient.invalidateQueries({
              queryKey: ["local-tailscale"],
            });
            void probe.refetch();
          }}
        >
          <RefreshCw
            className={probe.isFetching ? "animate-spin" : undefined}
          />
          Refresh
        </Button>
      </div>

      <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-8 pb-8">
        {local === null ? (
          <Empty>Checking Tailscale…</Empty>
        ) : !local.installed ? (
          <Empty>
            <p>
              Tailscale isn't installed on this computer, so there is no tailnet
              to look at.
            </p>
            <div className="mt-4 flex justify-center">
              <HelpButton label="Get Tailscale" />
            </div>
          </Empty>
        ) : !local.running ? (
          <Empty>
            <p>
              Tailscale isn't signed in on this computer. Sign in to see your
              devices.
            </p>
            <div className="mt-4 flex justify-center">
              <HelpButton label="How to sign in" />
            </div>
          </Empty>
        ) : addresses.length > 0 && probe.isPending ? (
          <Empty>
            <Loader2 className="mr-2 inline size-4 animate-spin" />
            Looking for Aircast devices…
          </Empty>
        ) : devices.length === 0 ? (
          <NoDevices
            summary={tailnetSummary(others, offline)}
            onFlash={onFlash}
          />
        ) : (
          devices.map((peer) => <DeviceRow key={peer.dnsName} peer={peer} />)
        )}

        {devices.length > 0 && tailnetSummary(others, offline) ? (
          <p className="px-1 text-xs text-muted-foreground">
            {tailnetSummary(others, offline)}
          </p>
        ) : null}
      </div>
    </div>
  );
}
