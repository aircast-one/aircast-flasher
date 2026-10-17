import { useState } from "react";
import { Check, Copy, Download, ExternalLink } from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { track } from "@/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Segmented } from "@/components/segmented";
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

function KeyLinks() {
  const local = useLocalTailscale();

  return (
    <div className="flex items-center gap-3">
      {local !== null && !local.installed ? (
        <Button
          type="button"
          variant="link"
          className="h-auto p-0"
          onClick={() => {
            track("tailscale_client_download_click");
            void openUrl(DOWNLOAD_URL);
          }}
        >
          <Download />
          Get Tailscale
        </Button>
      ) : null}
      <Button
        type="button"
        variant="link"
        className="h-auto p-0"
        onClick={() => {
          track("tailscale_keys_page_click");
          void openUrl(KEYS_URL);
        }}
      >
        Get a key
        <ExternalLink />
      </Button>
    </div>
  );
}

type Provider = "tailscale" | "headscale";

const PROVIDERS: { value: Provider; label: string }[] = [
  { value: "tailscale", label: "Tailscale" },
  { value: "headscale", label: "Headscale" },
];

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
  const pickProvider = (provider: Provider) => {
    onSelfHosted(provider === "headscale");
    if (provider === "tailscale") onControlServer("");
  };

  return (
    <>
      <Segmented
        label="Control server"
        options={PROVIDERS}
        value={selfHosted ? "headscale" : "tailscale"}
        onChange={pickProvider}
        className="self-start"
      />

      {selfHosted ? (
        <div className="flex flex-col gap-2">
          <Label htmlFor="control-server">Control server URL</Label>
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
              Tailscale, not your server.
            </p>
          ) : !controlServerValid ? (
            <p id="control-server-error" className="text-sm text-destructive">
              Enter a full URL starting with http:// or https:// — until then
              remote access is skipped.
            </p>
          ) : null}
        </div>
      ) : null}

      <div className="flex flex-col gap-2">
        <div className="flex items-center justify-between gap-3">
          <Label htmlFor="auth-key">Pre-auth key</Label>
          {selfHosted ? null : <KeyLinks />}
        </div>
        <Input
          id="auth-key"
          type="password"
          value={authKey}
          onChange={(e) => onAuthKey(e.currentTarget.value)}
          placeholder={
            selfHosted ? "Pre-auth key from your control server" : "tskey-auth-…"
          }
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          aria-describedby="auth-key-help"
        />
        <p id="auth-key-help" className="text-sm text-muted-foreground">
          Used once on first boot, so pick a short-expiry or ephemeral key.
          {selfHosted ? " Create one on your server with:" : null}
        </p>
        {selfHosted ? <HeadscaleCommand /> : null}
      </div>
    </>
  );
}
