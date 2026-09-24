import { useEffect, useState } from "react";
import { Eye, EyeOff, KeyRound } from "lucide-react";

import { identifyPublicKey } from "@/api";
import type { SshKeyIdentity } from "@/types";

import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { MIN_DEVICE_PASSWORD } from "@/components/step-network.validation";
import type { SshMode, SshPublicKey } from "@/types";

const SSH_MODES: { value: SshMode; label: string }[] = [
  { value: "password", label: "Password" },
  { value: "key-only", label: "SSH key" },
  { value: "disabled", label: "Disabled" },
];

// This key is about to turn password login off, and a wrong one means
// re-flashing the card to get back in. The input truncates before the comment,
// which is the only part a human recognises — so name the key, and print the
// fingerprint `ssh-keygen -lf` prints so it can actually be compared.
function KeyIdentity({ sshKey }: { sshKey: string }) {
  const [identity, setIdentity] = useState<SshKeyIdentity | null>(null);

  useEffect(() => {
    const key = sshKey.trim();
    if (key === "") {
      setIdentity(null);
      return;
    }
    let cancelled = false;
    identifyPublicKey(key)
      .then((id) => {
        if (!cancelled) setIdentity(id);
      })
      .catch(() => undefined);
    return () => {
      cancelled = true;
    };
  }, [sshKey]);

  if (identity === null) return null;

  return (
    <div className="flex items-start gap-2 rounded-md bg-muted/50 px-2.5 py-1.5">
      <KeyRound className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
      <div className="flex min-w-0 flex-col">
        <span className="truncate text-xs">
          {identity.comment === "" ? (
            <span className="text-muted-foreground">
              {identity.algorithm} · no comment
            </span>
          ) : (
            <>
              <span className="font-medium">{identity.comment}</span>
              <span className="text-muted-foreground">
                {" · "}
                {identity.algorithm}
              </span>
            </>
          )}
        </span>
        <span className="truncate font-mono text-xs text-muted-foreground">
          {identity.fingerprint}
        </span>
      </div>
    </div>
  );
}

export interface DeviceAccessProps {
  sshMode: SshMode;
  onSshMode: (v: SshMode) => void;
  sshKey: string;
  onSshKey: (v: string) => void;
  detectedKeys: SshPublicKey[];
  onChooseKeyFile: () => void;
  devicePassword: string;
  onDevicePassword: (v: string) => void;
  sshKeyInvalid: boolean;
  devicePasswordWeak: boolean;
}

export function DeviceAccessSection({
  sshMode,
  onSshMode,
  sshKey,
  onSshKey,
  detectedKeys,
  onChooseKeyFile,
  devicePassword,
  onDevicePassword,
  sshKeyInvalid,
  devicePasswordWeak,
}: DeviceAccessProps) {
  const [showDevicePassword, setShowDevicePassword] = useState(false);

  return (
    <>
      <p className="text-sm text-muted-foreground">
        Default login is <span className="font-medium text-foreground">pi</span>{" "}
        / <span className="font-medium text-foreground">raspberry</span> — set a
        key or password before deploying.
      </p>
      <div className="flex gap-2">
        {SSH_MODES.map((m) => (
          <Button
            key={m.value}
            type="button"
            variant={sshMode === m.value ? "default" : "secondary"}
            onClick={() => onSshMode(m.value)}
            aria-pressed={sshMode === m.value}
          >
            {m.label}
          </Button>
        ))}
      </div>

      {sshMode === "key-only" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="ssh-key">Public key</Label>
          <div className="flex flex-wrap gap-2">
            {detectedKeys.map((k) => (
              <Button
                key={k.label}
                type="button"
                variant="secondary"
                size="sm"
                onClick={() => onSshKey(k.contents)}
                aria-pressed={sshKey.trim() === k.contents.trim()}
              >
                Use {k.label}
              </Button>
            ))}
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={onChooseKeyFile}
            >
              Choose file…
            </Button>
          </div>
          <Input
            id="ssh-key"
            className="font-mono"
            value={sshKey}
            onChange={(e) => onSshKey(e.currentTarget.value)}
            placeholder="ssh-ed25519 AAAA… you@host"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            aria-invalid={sshKeyInvalid}
            aria-describedby="ssh-key-help"
          />
          <KeyIdentity sshKey={sshKey} />
          <p id="ssh-key-help" className="text-sm text-muted-foreground">
            {sshKeyInvalid ? (
              <span className="text-destructive">
                That doesn't look like an SSH public key — it should start with
                ssh-ed25519, ssh-rsa, …
              </span>
            ) : sshKey.trim() === "" ? (
              <>
                Add a key to turn password login off. Blank keeps the image
                default —{" "}
                <span className="font-medium text-foreground">
                  pi / raspberry
                </span>{" "}
                stays usable.
              </>
            ) : (
              <>
                Password login is off on the device — check the key above is
                the one you hold, or you'll need to re-flash to get back in.
              </>
            )}
          </p>
        </div>
      )}

      {sshMode === "password" && (
        <div className="flex flex-col gap-2">
          <Label htmlFor="device-password">Device password</Label>
          <div className="flex gap-2">
            <Input
              id="device-password"
              type={showDevicePassword ? "text" : "password"}
              value={devicePassword}
              onChange={(e) => onDevicePassword(e.currentTarget.value)}
              placeholder="New password for the pi account"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              aria-invalid={devicePasswordWeak}
              aria-describedby="device-password-help"
              className="flex-1"
            />
            <Button
              type="button"
              variant="secondary"
              size="icon"
              onClick={() => setShowDevicePassword((v) => !v)}
              aria-label={
                showDevicePassword ? "Hide password" : "Show password"
              }
            >
              {showDevicePassword ? <EyeOff /> : <Eye />}
            </Button>
          </div>
          <p id="device-password-help" className="text-sm text-muted-foreground">
            {devicePasswordWeak ? (
              <span className="text-destructive">
                Use at least {MIN_DEVICE_PASSWORD} characters.
              </span>
            ) : (
              <>
                Sets the <span className="font-medium text-foreground">pi</span>{" "}
                account password, replacing the default. Leave blank to keep the
                image default.
              </>
            )}
          </p>
        </div>
      )}

      {sshMode === "disabled" && (
        <p className="text-sm text-muted-foreground">
          SSH is turned off. Manage this drone from its web dashboard instead.
        </p>
      )}
    </>
  );
}
