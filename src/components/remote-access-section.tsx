import { Plus } from "lucide-react";

import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";

export interface RemoteAccessProps {
  controlServer: string;
  onControlServer: (v: string) => void;
  authKey: string;
  onAuthKey: (v: string) => void;
  selfHosted: boolean;
  onSelfHosted: (v: boolean) => void;
  controlServerValid: boolean;
  controlServerMissing: boolean;
  missingAuthKey: boolean;
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
  missingAuthKey,
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
              Enter a full URL starting with http:// or https://.
            </p>
          ) : null}
        </div>
      )}

      <div className="flex flex-col gap-2">
        <div className="flex items-baseline justify-between">
          <Label htmlFor="auth-key">Pre-auth key</Label>
          {!selfHosted && (
            <a
              href="https://login.tailscale.com/admin/settings/keys"
              target="_blank"
              rel="noopener noreferrer"
              className="text-sm text-muted-foreground underline hover:text-foreground"
            >
              Get a key
            </a>
          )}
        </div>
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
          aria-invalid={missingAuthKey}
          aria-describedby="auth-key-help"
        />
        {missingAuthKey ? (
          <p id="auth-key-help" className="text-sm text-destructive">
            A control server needs a pre-auth key — without one, nothing is
            written. Clear the server to use Tailscale instead.
          </p>
        ) : (
          <p id="auth-key-help" className="text-sm text-muted-foreground">
            The device joins automatically on first boot — no sign-in. The key is
            written to the card, so use a{" "}
            <span className="font-medium text-foreground">
              short-expiry or ephemeral
            </span>{" "}
            key. Leave blank to set it up later from the dashboard.
          </p>
        )}
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
