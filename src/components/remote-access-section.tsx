import { useState } from "react";
import { Check, Copy, Download, ExternalLink, Plus } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { track } from "@/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useLocalTailscale } from "@/lib/use-local-tailscale";

const KEYS_URL = "https://login.tailscale.com/admin/settings/keys";
const DOWNLOAD_URL = "https://aircast.one/docs/software/tailscale";
const HEADSCALE_COMMAND =
  "headscale preauthkeys create --user <you> --expiration 1h";

const COPIED_FEEDBACK_MS = 2000;

function HeadscaleCommand() {
  const [copied, setCopied] = useState(false);
  return (
    <div className="flex items-center gap-2 rounded-md bg-muted/50 px-2.5 py-1.5">
      <code className="min-w-0 flex-1 truncate text-xs">
        {HEADSCALE_COMMAND}
      </code>
      <Button
        type="button"
        size="sm"
        variant="ghost"
        aria-label="Copy command"
        onClick={() => {
          navigator.clipboard
            ?.writeText(HEADSCALE_COMMAND)
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
  );
}

function WhereTheKeyComesFrom({ selfHosted }: { selfHosted: boolean }) {
  const local = useLocalTailscale();

  if (selfHosted) return <HeadscaleCommand />;

  return (
    <div className="flex flex-wrap gap-2">
      <Button
        type="button"
        size="sm"
        variant="secondary"
        onClick={() => {
          track("tailscale_keys_page_click");
          void openUrl(KEYS_URL);
        }}
      >
        <ExternalLink className="size-4" />
        Get a key
      </Button>
      {local !== null && !local.installed ? (
        <Button
          type="button"
          size="sm"
          variant="secondary"
          onClick={() => {
            track("tailscale_client_download_click");
            void openUrl(DOWNLOAD_URL);
          }}
        >
          <Download className="size-4" />
          Get Tailscale
        </Button>
      ) : null}
    </div>
  );
}

export interface RemoteAccessProps {
  controlServer: string;
  onControlServer: (v: string) => void;
  authKey: string;
  onAuthKey: (v: string) => void;
  selfHosted: boolean;
  onSelfHosted: (v: boolean) => void;
  controlServerValid: boolean;
  controlServerMissing: boolean;
}

export function RemoteAccessSection({
  controlServer,
  onControlServer,
  authKey,
  onAuthKey,
  selfHosted,
  onSelfHosted,
  controlServerValid,
  controlServerMissing,
}: RemoteAccessProps) {
  function switchToTailscale() {
    onControlServer("");
    onSelfHosted(false);
  }

  return (
    <div className="flex flex-col gap-4">
      <p className="text-sm text-muted-foreground">
        Reach this drone from anywhere over a private{" "}
        {selfHosted ? "Headscale" : "Tailscale"} network.
      </p>

      {selfHosted && (
        <div className="flex flex-col gap-2">
          <div className="flex items-baseline justify-between">
            <Label htmlFor="control-server">Control server (Headscale)</Label>
            <button
              type="button"
              onClick={switchToTailscale}
              className="text-sm text-muted-foreground underline hover:text-foreground"
            >
              Use Tailscale
            </button>
          </div>
          <Input
            id="control-server"
            value={controlServer}
            onChange={(e) => onControlServer(e.currentTarget.value)}
            placeholder="https://headscale.example.com"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            aria-invalid={!controlServerValid || controlServerMissing}
            aria-describedby="control-server-error"
          />
          {controlServerMissing ? (
            <p id="control-server-error" className="text-sm text-destructive">
              Enter your control server URL — without it the key is sent to
              Tailscale, not your server. Or switch to Tailscale.
            </p>
          ) : !controlServerValid ? (
            <p id="control-server-error" className="text-sm text-destructive">
              Enter a full URL starting with http:// or https:// — until then
              remote access is skipped.
            </p>
          ) : null}
        </div>
      )}

      <div className="flex flex-col gap-2">
        <Label htmlFor="auth-key">Pre-auth key</Label>
        <Input
          id="auth-key"
          type="password"
          value={authKey}
          onChange={(e) => onAuthKey(e.currentTarget.value)}
          placeholder={
            selfHosted
              ? "Pre-auth key from your control server"
              : "tskey-auth-…"
          }
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          aria-describedby="auth-key-help"
        />
        <p id="auth-key-help" className="text-sm text-muted-foreground">
          Written to the card and used once on first boot, so prefer a{" "}
          <span className="font-medium text-foreground">
            short-expiry or ephemeral
          </span>{" "}
          key. Leave blank to sign the device in after boot instead.
        </p>
        <WhereTheKeyComesFrom selfHosted={selfHosted} />
      </div>

      {!selfHosted && (
        <button
          type="button"
          onClick={() => onSelfHosted(true)}
          className="flex items-center gap-1.5 self-start text-sm text-muted-foreground hover:text-foreground"
        >
          <Plus className="size-4" />
          Use a self-hosted control server (Headscale)
        </button>
      )}
    </div>
  );
}
