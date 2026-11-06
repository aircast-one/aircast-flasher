import { useCallback, useEffect, useRef, useState } from "react";
import {
  ArrowRight,
  Check,
  Copy,
  Download,
  Loader2,
  Network,
  Wifi,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  joinWifi,
  probeDevice,
  setDeviceControlServer,
  tailscaleLogin,
  tailscaleStatus,
  track,
} from "@/api";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/format";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import {
  type FoundDevice,
  setupUrl,
  tailnetDevice,
} from "@/components/device-address";
import type { LocalTailscale, TailnetPeer } from "@/types";

const PROBE_INTERVAL_MS = 3000;
const HOTSPOT_AFTER_ROUNDS = 5;
const LOGIN_POLL_INTERVAL_MS = 3000;
const TAILSCALE_DOCS_URL = "https://aircast.one/docs/software/tailscale";
const HOTSPOT_URL = "http://10.42.0.1";
const HOTSPOT_PASSWORD = "raspberry";
export interface RestoredCard {
  host: string;
  rejoins: boolean;
}

export const missingAfterMinutes = (restore: RestoredCard | null): number =>
  restore === null ? 10 : 15;

type JoinState =
  { phase: "idle" } | { phase: "joining" } | { phase: "failed"; error: string };

type LoginState =
  | { phase: "idle" }
  | { phase: "starting" }
  | { phase: "signing-in" }
  | { phase: "connected"; name: string }
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

const HOTSPOT_HELP =
  "You're on its hotspot, so it has no internet yet. Setup starts by joining it to your WiFi.";

const LOGIN_HELP: Record<LoginState["phase"], string> = {
  idle: "Opens the sign-in page in your browser, then authorizes this device.",
  starting:
    "Asking Tailscale for a sign-in link — this can take a few seconds.",
  "signing-in":
    "Finish signing in in your browser. This updates on its own; click again for a fresh link.",
  connected: "",
  failed: "",
};

const LOGIN_LABEL: Record<LoginState["phase"], string> = {
  idle: "Connect to Tailscale",
  starting: "Getting a sign-in link…",
  "signing-in": "Waiting for sign-in…",
  connected: "",
  failed: "Connect to Tailscale",
};

const selfHostedLabel = (phase: LoginState["phase"], server: string) =>
  phase === "idle" || phase === "failed"
    ? `Connect to ${server}`
    : LOGIN_LABEL[phase];

function localGap(local: LocalTailscale, deviceName: string): string | null {
  if (!local.installed)
    return "Tailscale isn't installed on this computer — you need it here too to reach the drone from another network.";
  if (!local.running)
    return "Tailscale is installed here but not signed in. Open it and sign in to the same account you use for the drone.";
  if (
    deviceName === "" ||
    local.magicDnsSuffix === "" ||
    deviceName.endsWith(`.${local.magicDnsSuffix}`)
  )
    return null;
  return `This computer is on a different tailnet (${local.magicDnsSuffix}). Sign in to the same account you used for the drone.`;
}

function LocalTailscaleRow({ deviceName }: { deviceName: string }) {
  const local = useLocalTailscale();

  if (local === null) return null;
  const gap = localGap(local, deviceName);
  if (gap === null) return null;

  return (
    <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2">
      <span className="min-w-0 flex-1 text-xs text-muted-foreground">
        {gap}
      </span>
      {local.installed ? null : (
        <Button
          type="button"
          size="sm"
          variant="secondary"
          className="shrink-0"
          onClick={() => {
            track("tailscale_client_download_click");
            void openUrl(TAILSCALE_DOCS_URL);
          }}
        >
          <Download className="size-4" />
          Get Tailscale
        </Button>
      )}
    </div>
  );
}

function useNewTailnetPeer(
  hostname: string,
  rejoins: boolean,
): TailnetPeer | null {
  const local = useLocalTailscale();
  const baseline = useRef<Set<string> | null>(null);
  const counts = (peer: TailnetPeer) => !rejoins || peer.online;

  if (baseline.current === null && local !== null) {
    baseline.current = new Set(
      local.peers.filter(counts).map((peer) => peer.dnsName),
    );
  }

  const known = baseline.current;
  if (local === null || known === null) return null;

  const expected = hostname.trim().toLowerCase();
  return (
    local.peers.find(
      (peer) =>
        counts(peer) &&
        !known.has(peer.dnsName) &&
        peer.hostName.toLowerCase().startsWith(expected),
    ) ?? null
  );
}

function RestoreNote({ host }: { host: string }) {
  return (
    <span id="finder-restore">
      This card is restoring {host}'s settings, so it restarts once more on
      first boot — allow a few extra minutes.
    </span>
  );
}

function TailscaleSignIn({ deviceUrl }: { deviceUrl: string }) {
  const local = useLocalTailscale();
  const [login, setLogin] = useState<LoginState>({ phase: "idle" });
  const selfHostedControl =
    local !== null && local.selfHosted ? local.controlUrl : "";

  const connected = useCallback(
    (name: string) => setLogin({ phase: "connected", name }),
    [],
  );

  useEffect(() => {
    if (login.phase !== "idle") return;
    let cancelled = false;
    tailscaleStatus(deviceUrl)
      .then((s) => {
        if (s.connected && !cancelled) connected(s.name);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [deviceUrl, login.phase, connected]);

  useEffect(() => {
    if (login.phase !== "signing-in") return;
    let cancelled = false;
    const timer = setInterval(() => {
      void tailscaleStatus(deviceUrl)
        .then((s) => {
          if (s.connected && !cancelled) {
            track("tailscale_login_connected");
            connected(s.name);
          }
        })
        .catch(() => undefined);
    }, LOGIN_POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [deviceUrl, login.phase, connected]);

  function handleLogin() {
    setLogin({ phase: "starting" });
    track("tailscale_login_attempt", { selfHosted: selfHostedControl !== "" });
    (selfHostedControl === ""
      ? Promise.resolve()
      : setDeviceControlServer(deviceUrl, selfHostedControl)
    )
      .then(() => tailscaleLogin(deviceUrl))
      .then((authUrl) =>
        authUrl === ""
          ? tailscaleStatus(deviceUrl).then((s) => connected(s.name))
          : openUrl(authUrl).then(() => setLogin({ phase: "signing-in" })),
      )
      .catch((err) => {
        const error = errorMessage(err, "Couldn't start Tailscale sign-in.");
        track("tailscale_login_failed", { error });
        setLogin({ phase: "failed", error });
      });
  }

  return (
    <div className="flex flex-col gap-2">
      {login.phase === "connected" ? (
        <p className="flex items-center gap-2 text-xs">
          <Check className="size-4 shrink-0 text-green-500 dark:text-green-400" />
          <span>
            On your tailnet
            {login.name === "" ? null : (
              <>
                {" as "}
                <span className="font-mono">{login.name}</span>
              </>
            )}
          </span>
        </p>
      ) : (
        <div className="flex flex-col gap-1">
          <Button
            type="button"
            size="sm"
            variant="ghost"
            className="self-start"
            disabled={login.phase === "starting"}
            onClick={handleLogin}
          >
            {login.phase === "idle" || login.phase === "failed" ? (
              <Network className="size-4" />
            ) : (
              <Loader2 className="size-4 animate-spin" />
            )}
            {selfHostedControl === ""
              ? LOGIN_LABEL[login.phase]
              : selfHostedLabel(
                  login.phase,
                  selfHostedControl.replace(/^https?:\/\//, ""),
                )}
          </Button>
          <p
            className={
              login.phase === "failed"
                ? "text-xs text-destructive"
                : "text-xs text-muted-foreground"
            }
          >
            {login.phase === "failed" ? login.error : LOGIN_HELP[login.phase]}
          </p>
        </div>
      )}
      <LocalTailscaleRow
        deviceName={login.phase === "connected" ? login.name : ""}
      />
    </div>
  );
}

export function useFoundDevice(
  hostname: string,
  fightMode: boolean,
  restore: RestoredCard | null,
): { found: FoundDevice | null; rounds: number } {
  const [hit, setHit] = useState<{ url: string; ip: string } | null>(null);
  const [rounds, setRounds] = useState(0);
  const tailnetPeer = useNewTailnetPeer(hostname, restore?.rejoins ?? false);
  const lanUrl = `http://${hostname}.local`;

  useEffect(() => {
    if (hit !== null || fightMode) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;

    const round = async () => {
      const results = await Promise.all(
        [lanUrl, HOTSPOT_URL].map((url) =>
          probeDevice(url).then(
            (ip) => (ip === null ? null : { url, ip }),
            () => null,
          ),
        ),
      );
      if (cancelled) return;
      const answered = results.find((r) => r !== null) ?? null;
      if (answered !== null) {
        setHit(answered);
        track("device_found", { via: answered.url === HOTSPOT_URL ? "hotspot" : "lan" });
        return;
      }
      setRounds((n) => n + 1);
      timer = setTimeout(() => void round(), PROBE_INTERVAL_MS);
    };

    void round();
    return () => {
      cancelled = true;
      clearTimeout(timer);
    };
  }, [hit, lanUrl, fightMode]);

  const found: FoundDevice | null =
    hit?.url === HOTSPOT_URL
      ? { url: hit.url, address: hit.ip, via: "hotspot", note: HOTSPOT_HELP }
      : hit !== null
        ? { url: `http://${hit.ip}`, address: hit.ip, via: "lan", note: `Also answers as ${hostname}.local.` }
        : tailnetPeer !== null
          ? tailnetDevice(tailnetPeer, hostname)
          : null;
  return { found, rounds };
}

export function DeviceOnline({ found }: { found: FoundDevice }) {
  return (
    <div className="flex flex-col gap-1">
      <CopyableAddress address={found.address} />
      {found.note === "" ? null : (
        <span className="text-xs text-muted-foreground">{found.note}</span>
      )}
    </div>
  );
}

export function ContinueSetup({
  found,
  hostname,
}: {
  found: FoundDevice;
  hostname: string;
}) {
  return (
    <div className="flex flex-col gap-3">
      <Button
        type="button"
        className="self-start"
        onClick={() => {
          track("setup_continue_click", { via: found.via });
          void openUrl(setupUrl(found.url));
        }}
      >
        Continue setup
        <ArrowRight className="size-4" />
      </Button>
      <span className="text-xs text-muted-foreground">
        Opens {hostname.trim()} in your browser. Nothing to install.
      </span>
      {found.via === "lan" ? <TailscaleSignIn deviceUrl={found.url} /> : null}
    </div>
  );
}

export function DeviceWaiting({
  hostname,
  remoteEnrolled,
  fightMode,
  restore,
  rounds,
}: {
  hostname: string;
  remoteEnrolled: boolean;
  fightMode: boolean;
  restore: RestoredCard | null;
  rounds: number;
}) {
  const [join, setJoin] = useState<JoinState>({ phase: "idle" });

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

  if (fightMode)
    return (
      <div className="flex flex-col gap-1 text-xs text-muted-foreground">
        {restore === null ? null : <RestoreNote host={restore.host} />}
        <span>
          Fight mode turns off its hotspot and blocks every other network, so it
          only shows up once it{" "}
          {restore?.rejoins
            ? `rejoins your private network as ${restore.host}.`
            : "joins your private network."}
        </span>
      </div>
    );

  return (
    <div className="flex flex-col gap-2 text-xs text-muted-foreground">
      <span>
        {restore === null
          ? "The first start takes a few minutes and it restarts once — don't unplug it."
          : `The first start takes a few minutes and it restarts twice, the second time to restore ${restore.host}'s settings — don't unplug it.`}{" "}
        {restore?.rejoins
          ? `It rejoins your private network as ${restore.host}, with the identity from the backup.`
          : remoteEnrolled
            ? "It shows up here from this network or your private one."
            : "Keep this computer on the same network."}
      </span>
      {rounds >= HOTSPOT_AFTER_ROUNDS ? (
        <div className="flex flex-col gap-2">
          <span>
            If it couldn't find your WiFi it starts a hotspot named{" "}
            <span className="font-mono">{hostname}</span> (password{" "}
            <span className="font-mono">{HOTSPOT_PASSWORD}</span>) — joining it
            moves this computer off your network.
          </span>
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
      ) : null}
    </div>
  );
}
