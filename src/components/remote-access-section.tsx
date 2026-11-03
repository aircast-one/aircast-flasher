import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import {
  Check,
  Copy,
  Download,
  ExternalLink,
  TriangleAlert,
} from "lucide-react";
import { openUrl } from "@tauri-apps/plugin-opener";

import { headscaleUsers, track } from "@/api";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Segmented } from "@/components/segmented";
import {
  isHeadscaleApiKey,
  mintsKeys,
  reusedKeyNotice,
} from "@/components/step-network.validation";
import { errorMessage } from "@/lib/format";
import { useLocalTailscale } from "@/lib/use-local-tailscale";
import type { KeyWrite } from "@/types";

const KEYS_URL = "https://login.tailscale.com/admin/settings/keys";
const DOWNLOAD_URL = "https://aircast.one/docs/software/tailscale";
const HEADSCALE_COMMAND = "headscale apikeys create --expiration 365d";

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
        Get a token
        <ExternalLink />
      </Button>
    </div>
  );
}

const keyHelp = (selfHosted: boolean, minting: boolean): string =>
  selfHosted
    ? minting
      ? "Each card gets its own one-time key, valid for 90 days."
      : "Paste an API key and each card gets its own one-time key. Make one on your server with:"
    : minting
      ? "Each card gets its own one-time key, valid for 90 days. The token itself lasts at most 90 days — when Tailscale refuses it, generate a new one."
      : "Paste an API access token and each card gets its own one-time key: Get a token opens Tailscale's Keys page, then choose Generate access token. Tokens last at most 90 days. A pasted pre-auth key must still be valid when the drone first boots, and not ephemeral.";

function HeadscaleUserPicker({
  controlServer,
  apiKey,
  value,
  onChange,
}: {
  controlServer: string;
  apiKey: string;
  value: string;
  onChange: (v: string) => void;
}) {
  const users = useQuery({
    queryKey: ["headscale-users", controlServer.trim(), apiKey.trim()],
    queryFn: () => headscaleUsers(controlServer, apiKey),
    enabled: /^https?:\/\/.+/.test(controlServer.trim()),
    retry: false,
  });
  const names = users.data;

  useEffect(() => {
    if (names === undefined || names.includes(value)) return;
    const next = names.length === 1 ? names[0] : "";
    if (next !== value) onChange(next);
  }, [names, value, onChange]);

  return (
    <div className="flex flex-col gap-2">
      <Label htmlFor="headscale-user">Headscale user</Label>
      <Select
        value={value}
        onValueChange={(v) => onChange(String(v))}
        disabled={names === undefined}
      >
        <SelectTrigger
          id="headscale-user"
          aria-label="Headscale user"
          aria-invalid={value === "" && names !== undefined}
          className="w-full"
        >
          <SelectValue>
            {(v) =>
              users.isLoading
                ? "Loading users…"
                : String(v) === ""
                  ? "Pick the user drones join as"
                  : String(v)
            }
          </SelectValue>
        </SelectTrigger>
        <SelectContent>
          {(names ?? []).map((name) => (
            <SelectItem key={name} value={name}>
              {name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {users.isError ? (
        <p id="headscale-user-error" className="text-sm text-destructive">
          {errorMessage(users.error, "Couldn't list the users on Headscale.")}
        </p>
      ) : null}
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
  keyWrittenTo: KeyWrite | null;
  selfHosted: boolean;
  onSelfHosted: (v: boolean) => void;
  headscaleUser: string;
  onHeadscaleUser: (v: string) => void;
  controlServerValid: boolean;
  controlServerMissing: boolean;
}

export function RemoteAccessSection({
  controlServer,
  onControlServer,
  authKey,
  onAuthKey,
  keyWrittenTo,
  selfHosted,
  onSelfHosted,
  headscaleUser,
  onHeadscaleUser,
  controlServerValid,
  controlServerMissing,
}: RemoteAccessProps) {
  const pickProvider = (provider: Provider) => {
    onSelfHosted(provider === "headscale");
  };
  const reused = reusedKeyNotice(keyWrittenTo, authKey);
  const minting = mintsKeys(authKey);
  const needsUser = selfHosted && isHeadscaleApiKey(authKey);

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
          <Label htmlFor="auth-key">
            {selfHosted
              ? "API key or pre-auth key"
              : "API token or pre-auth key"}
          </Label>
          {selfHosted ? null : <KeyLinks />}
        </div>
        <Input
          id="auth-key"
          type="password"
          value={authKey}
          onChange={(e) => onAuthKey(e.currentTarget.value)}
          placeholder={
            selfHosted
              ? "hskey-api-… or hskey-auth-…"
              : "tskey-api-… or tskey-auth-…"
          }
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          aria-describedby={
            reused === null ? "auth-key-help" : "auth-key-reused auth-key-help"
          }
        />
        {reused === null ? null : (
          <div className="flex items-start gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-3 py-2">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400" />
            <p id="auth-key-reused" className="text-sm text-muted-foreground">
              {reused}
            </p>
          </div>
        )}
        <p id="auth-key-help" className="text-sm text-muted-foreground">
          {keyHelp(selfHosted, minting)}
        </p>
        {selfHosted && !minting ? <HeadscaleCommand /> : null}
      </div>

      {needsUser ? (
        <HeadscaleUserPicker
          controlServer={controlServer}
          apiKey={authKey}
          value={headscaleUser}
          onChange={onHeadscaleUser}
        />
      ) : null}
    </>
  );
}
