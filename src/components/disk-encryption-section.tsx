import { useState } from "react";
import {
  Check,
  Copy,
  Download,
  Loader2,
  Plus,
  ShieldCheck,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { fetchTangAdv, saveRecoveryFile } from "@/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import type { TangServer } from "@/types";
import { hasUrl, isCaptured } from "@/components/encryption";

const COPIED_FEEDBACK_MS = 2000;

export interface DiskEncryptionProps {
  enabled: boolean;
  onEnabled: (v: boolean) => void;
  servers: TangServer[];
  onServers: (v: TangServer[]) => void;
  threshold: number;
  onThreshold: (v: number) => void;
  recoveryKey: string;
  recoverySaved: boolean;
  onRecoverySaved: (v: boolean) => void;
  wifiUplink: boolean;
  hostname: string;
}

export function DiskEncryptionSection(props: DiskEncryptionProps) {
  return (
    <div className="flex flex-col gap-4">
      <TangServers
        servers={props.servers}
        onServers={props.onServers}
        threshold={props.threshold}
        onThreshold={props.onThreshold}
      />
      {props.wifiUplink ? <WifiPskNote /> : null}
      <RecoveryKeyField
        recoveryKey={props.recoveryKey}
        recoverySaved={props.recoverySaved}
        onRecoverySaved={props.onRecoverySaved}
        servers={props.servers}
        threshold={props.threshold}
        hostname={props.hostname}
      />
    </div>
  );
}

function TangServers({
  servers,
  onServers,
  threshold,
  onThreshold,
}: Pick<
  DiskEncryptionProps,
  "servers" | "onServers" | "threshold" | "onThreshold"
>) {
  const [busy, setBusy] = useState(false);
  const [errors, setErrors] = useState<Record<number, string>>({});

  const setUrl = (i: number, url: string) => {
    onServers(servers.map((s, j) => (j === i ? { url, adv: "" } : s)));
    setErrors((e) => ({ ...e, [i]: "" }));
  };
  const addRow = () => onServers([...servers, { url: "", adv: "" }]);
  const removeRow = (i: number) =>
    onServers(servers.filter((_, j) => j !== i));

  const captureAll = () => {
    setBusy(true);
    setErrors({});
    Promise.all(
      servers.map(async (s) => {
        if (s.url.trim() === "") return { server: s, error: null as string | null };
        try {
          return { server: { ...s, adv: await fetchTangAdv(s.url) }, error: null };
        } catch (e) {
          const error = typeof e === "string" ? e : "Could not reach this server.";
          return { server: { ...s, adv: "" }, error };
        }
      }),
    ).then((results) => {
      onServers(results.map((r) => r.server));
      setErrors(
        Object.fromEntries(
          results.flatMap((r, i) => (r.error ? [[i, r.error]] : [])),
        ),
      );
      setBusy(false);
    });
  };

  const urlCount = servers.filter(hasUrl).length;

  return (
    <div className="flex flex-col gap-2">
      <Label>Key servers (Tang)</Label>
      <p className="text-sm text-muted-foreground">
        Each time a drone starts, it gets its unlock from a key server on your
        network. That's what keeps a seized drone locked: away from your network
        it can't unlock, even powered on. If the server is ever down, the
        recovery key below unlocks the card by hand.
      </p>
      <div className="flex flex-col gap-2">
        {servers.map((s, i) => (
          <div key={i} className="flex flex-col gap-1">
            <div className="flex items-start gap-2">
              <Input
                className="flex-1"
                placeholder="http://10.0.0.5"
                value={s.url}
                aria-label={`Tang server ${i + 1}`}
                onChange={(e) => setUrl(i, e.target.value)}
              />
              {isCaptured(s) ? (
                <span
                  className="flex items-center gap-1 whitespace-nowrap px-1 text-sm text-green-600 dark:text-green-400"
                  title="Advertisement captured"
                >
                  <ShieldCheck className="size-4" />
                </span>
              ) : null}
              {servers.length > 1 ? (
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  aria-label={`Remove server ${i + 1}`}
                  onClick={() => removeRow(i)}
                >
                  <Trash2 className="size-4" />
                </Button>
              ) : null}
            </div>
            {errors[i] ? (
              <p className="text-sm text-destructive" role="alert">
                {errors[i]}
              </p>
            ) : null}
          </div>
        ))}
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button
          type="button"
          variant="secondary"
          disabled={busy || urlCount === 0}
          onClick={captureAll}
        >
          {busy ? <Loader2 className="size-4 animate-spin" /> : null}
          Test &amp; capture
        </Button>
        <Button type="button" variant="ghost" onClick={addRow}>
          <Plus className="size-4" />
          Add a server
        </Button>
      </div>
      <KeyServerSetup />
      {urlCount > 1 ? (
        <div className="flex items-center gap-2 pt-1">
          <Label htmlFor="threshold" className="whitespace-nowrap">
            Servers needed to unlock
          </Label>
          <Input
            id="threshold"
            type="number"
            min={1}
            max={urlCount}
            className="w-20"
            value={threshold}
            onChange={(e) => {
              const n = Number(e.target.value);
              onThreshold(Math.max(1, Math.min(urlCount, Number.isFinite(n) ? n : 1)));
            }}
          />
          <span className="text-sm text-muted-foreground">of {urlCount}</span>
        </div>
      ) : null}
    </div>
  );
}

const KEY_SERVER_PORT = 7500;
const KEY_SERVER_SETUP = [
  "sudo apt-get install -y tang",
  "sudo mkdir -p /etc/systemd/system/tangd.socket.d",
  `printf '[Socket]\\nListenStream=\\nListenStream=${KEY_SERVER_PORT}\\n' | sudo tee /etc/systemd/system/tangd.socket.d/port.conf`,
  "sudo systemctl daemon-reload",
  "sudo systemctl enable --now tangd.socket",
].join("\n");

function KeyServerSetup() {
  const [open, setOpen] = useState(false);
  const [copied, setCopied] = useState(false);

  if (!open) {
    return (
      <Button
        type="button"
        variant="link"
        className="h-auto self-start p-0 text-sm"
        onClick={() => setOpen(true)}
      >
        Don't have a key server? Set one up in one step
      </Button>
    );
  }

  return (
    <div className="flex flex-col gap-2 rounded-md border border-border px-3 py-2.5">
      <p className="text-sm">
        On any always-on Debian, Ubuntu or Raspberry Pi OS box on the drones'
        network, run:
      </p>
      <div className="flex items-start gap-2 rounded-md bg-muted/50 px-2.5 py-2">
        <pre className="min-w-0 flex-1 whitespace-pre-wrap break-all text-xs">
          {KEY_SERVER_SETUP}
        </pre>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-label="Copy setup commands"
          onClick={() => {
            navigator.clipboard
              ?.writeText(KEY_SERVER_SETUP)
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
      </div>
      <p className="text-sm text-muted-foreground">
        Then enter <code className="text-xs">http://&lt;that box's address&gt;:{KEY_SERVER_PORT}</code>{" "}
        above and press Test &amp; capture. You only do this once — the flasher
        remembers it for every card after.
      </p>
    </div>
  );
}

function WifiPskNote() {
  return (
    <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2">
      <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
      <p className="text-sm text-muted-foreground">
        This is a Wi-Fi device: the Wi-Fi password is stored unencrypted so the
        card can connect before it unlocks. The data partition (settings,
        credentials, tailnet key) stays protected. Use Ethernet if the Wi-Fi
        password itself must not leak.
      </p>
    </div>
  );
}

export function recoveryEscrowText(
  servers: TangServer[],
  threshold: number,
  hostname: string,
  recoveryKey: string,
): string {
  const host = hostname.trim() === "" ? "(unnamed device)" : hostname.trim();
  const urls = servers
    .filter(isCaptured)
    .map((s) => `  - ${s.url.trim()}`)
    .join("\n");
  return [
    "Aircast disk-encryption recovery key",
    "KEEP THIS SECRET. It unlocks the device's data partition by hand if the",
    "key servers are unreachable. It is NOT stored on the card.",
    "",
    `Device:    ${host}`,
    `Saved:     ${new Date().toISOString()}`,
    `Unlock:    ${threshold} of ${servers.filter(isCaptured).length} key server(s):`,
    urls,
    "",
    `Recovery key: ${recoveryKey}`,
    "",
  ].join("\n");
}

export function saveRecoveryKey(
  servers: TangServer[],
  threshold: number,
  hostname: string,
  recoveryKey: string,
): Promise<boolean> {
  const name = `${hostname.trim() || "aircast"}-recovery-key.txt`;
  return saveRecoveryFile(
    name,
    recoveryEscrowText(servers, threshold, hostname, recoveryKey),
  );
}

function RecoveryKeyField({
  recoveryKey,
  recoverySaved,
  onRecoverySaved,
  servers,
  threshold,
  hostname,
}: Pick<
  DiskEncryptionProps,
  | "recoveryKey"
  | "recoverySaved"
  | "onRecoverySaved"
  | "servers"
  | "threshold"
  | "hostname"
>) {
  const [copied, setCopied] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);

  const download = () => {
    setSaveError(null);
    saveRecoveryKey(servers, threshold, hostname, recoveryKey)
      .then((saved) => {
        if (saved) onRecoverySaved(true);
      })
      .catch((e) =>
        setSaveError(typeof e === "string" ? e : "Could not save the file."),
      );
  };

  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor="recovery-key">Recovery key</Label>
      <p className="text-sm text-muted-foreground">
        Break-glass passphrase to unlock the card by hand if the key servers are
        unreachable. It is wiped off the card after first boot, so this is your
        only copy — save it now.
      </p>
      <div className="flex items-start gap-2 rounded-md bg-muted/50 px-2.5 py-2">
        <code id="recovery-key" className="min-w-0 flex-1 break-all text-xs">
          {recoveryKey}
        </code>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          aria-label="Copy recovery key"
          onClick={() => {
            navigator.clipboard
              ?.writeText(recoveryKey)
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
      </div>
      <div className="flex flex-wrap items-center gap-2">
        <Button type="button" variant="secondary" onClick={download}>
          <Download className="size-4" />
          Download recovery key
        </Button>
        <Button
          type="button"
          variant={recoverySaved ? "default" : "ghost"}
          onClick={() => onRecoverySaved(!recoverySaved)}
        >
          {recoverySaved ? <Check className="size-4" /> : null}
          {recoverySaved ? "Saved" : "I've saved it elsewhere"}
        </Button>
      </div>
      {saveError ? (
        <p className="text-sm text-destructive" role="alert">
          {saveError}
        </p>
      ) : null}
    </div>
  );
}
