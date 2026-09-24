import { useCallback, useEffect, useRef, useState } from "react";
import {
  Check,
  Copy,
  Download,
  ExternalLink,
  Loader2,
  Network,
  Wifi,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import {
  joinWifi,
  openDeviceWindow,
  probeDevice,
  setDeviceControlServer,
  tailscaleLogin,
  tailscaleStatus,
  track,
} from "@/api";
import { Button } from "@/components/ui/button";
import { errorMessage } from "@/lib/format";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import type { LocalTailscale, TailnetPeer } from "@/types";

const PROBE_INTERVAL_MS = 3000;
const HOTSPOT_AFTER_ROUNDS = 5;
const LOGIN_POLL_INTERVAL_MS = 3000;
const TAILSCALE_DOCS_URL = "https://aircast.one/docs/software/tailscale";
const HOTSPOT_URL = "http://10.42.0.1";
const HOTSPOT_PASSWORD = "raspberry";

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
  "You're on the device's hotspot, so it has no internet yet. Connect it to WiFi on the device page, then come back to sign in to Tailscale.";

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

function useNewTailnetPeer(hostname: string): TailnetPeer | null {
  const local = useLocalTailscale();
  const baseline = useRef<Set<string> | null>(null);

  if (baseline.current === null && local !== null) {
    baseline.current = new Set(local.peers.map((peer) => peer.dnsName));
  }

  const known = baseline.current;
  if (local === null || known === null) return null;

  const expected = hostname.trim().toLowerCase();
  return (
    local.peers.find(
      (peer) =>
        !known.has(peer.dnsName) &&
        peer.hostName.toLowerCase().startsWith(expected),
    ) ?? null
  );
}

function DeviceActions({
  deviceUrl,
  hostname,
}: {
  deviceUrl: string;
  hostname: string;
}) {
  const onHotspot = deviceUrl === HOTSPOT_URL;
  const local = useLocalTailscale();
  const [login, setLogin] = useState<LoginState>({ phase: "idle" });
  const selfHostedControl =
    local !== null && local.selfHosted ? local.controlUrl : "";

  const connected = useCallback(
    (name: string) => setLogin({ phase: "connected", name }),
    [],
  );

  useEffect(() => {
    if (onHotspot || login.phase !== "idle") return;
    let cancelled = false;
    tailscaleStatus(deviceUrl)
      .then((s) => {
        if (s.connected && !cancelled) connected(s.name);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [deviceUrl, onHotspot, login.phase, connected]);

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

  const showLogin = !onHotspot && login.phase !== "connected";

  return (
    <div className="flex flex-col gap-2">
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          size="sm"
          onClick={() => void openDeviceWindow(deviceUrl, hostname)}
        >
          <ExternalLink className="size-4" />
          Open device page
        </Button>
        {showLogin ? (
          <Button
            type="button"
            size="sm"
            variant="secondary"
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
        ) : null}
      </div>
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
        <p
          className={
            login.phase === "failed"
              ? "text-xs text-destructive"
              : "text-xs text-muted-foreground"
          }
        >
          {onHotspot
            ? HOTSPOT_HELP
            : login.phase === "failed"
              ? login.error
              : LOGIN_HELP[login.phase]}
        </p>
      )}
      {onHotspot ? null : (
        <LocalTailscaleRow
          deviceName={login.phase === "connected" ? login.name : ""}
        />
      )}
    </div>
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
  const [rounds, setRounds] = useState(0);
  const [join, setJoin] = useState<JoinState>({ phase: "idle" });
  const tailnetPeer = useNewTailnetPeer(hostname);
  const lanUrl = `http://${hostname}.local`;

  useEffect(() => {
    if (foundUrl !== null) return;
    let cancelled = false;
    let timer: ReturnType<typeof setTimeout>;

    const round = async () => {
      const results = await Promise.all(
        [lanUrl, HOTSPOT_URL].map((url) =>
          probeDevice(url).then(
            (ok) => (ok ? url : null),
            () => null,
          ),
        ),
      );
      if (cancelled) return;
      const hit = results.find((r) => r !== null) ?? null;
      if (hit !== null) {
        setFoundUrl(hit);
        track("device_found", { via: hit === HOTSPOT_URL ? "hotspot" : "lan" });
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
        </div>
        <DeviceActions deviceUrl={foundUrl} hostname={hostname} />
      </div>
    );
  }

  if (tailnetPeer !== null) {
    return (
      <div className="flex w-full flex-col gap-1 rounded-xl border border-green-500/30 bg-green-500/5 px-4 py-3 text-left">
        <span className="text-xs font-medium tracking-wide text-green-600 uppercase dark:text-green-400">
          On your tailnet
        </span>
        <CopyableAddress address={tailnetPeer.dnsName} />
        <span className="text-xs text-muted-foreground">
          {tailnetPeer.hostName.toLowerCase() === hostname.trim().toLowerCase()
            ? `Reachable at ${tailnetPeer.ip} from anywhere on your tailnet.`
            : `Tailscale named it ${tailnetPeer.hostName} — ${hostname.trim()} was taken. Reachable at ${tailnetPeer.ip}.`}
          {" It isn't on this network, so "}
          <span className="font-mono">{hostname}.local</span>
          {" won't answer from here."}
        </span>
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
        <span className="flex items-center gap-1.5">
          <Network className="size-3.5 shrink-0" />
          {remoteEnrolled
            ? "It joins your private network on first boot, using the key you wrote to the card."
            : "Once it's online you can sign it in to Tailscale from here — keep this window open."}
        </span>
      </div>
      {rounds >= HOTSPOT_AFTER_ROUNDS ? (
        <div className="flex flex-col gap-2">
          <span className="text-xs text-muted-foreground">
            Not on this network yet. If it couldn't find your WiFi it starts a
            hotspot named <span className="font-mono">{hostname}</span>{" "}
            (password <span className="font-mono">{HOTSPOT_PASSWORD}</span>) —
            joining it moves this computer off your network.
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
