import { useState } from "react";
import { ChevronDown, Eye, EyeOff, Plus, RefreshCw, Wifi } from "lucide-react";

import { Button } from "@/components/ui/button";
import {
  Combobox,
  ComboboxContent,
  ComboboxEmpty,
  ComboboxInput,
  ComboboxItem,
  ComboboxList,
} from "@/components/ui/combobox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { StepShell } from "@/components/step-shell";
import {
  isInvalidSshKey,
  isValidControlServer,
  isValidHostname,
  isWeakDevicePassword,
  MIN_DEVICE_PASSWORD,
  needsAuthKey,
} from "@/components/step-network.validation";
import type { SshMode, SshPublicKey } from "@/types";

const SSH_MODES: { value: SshMode; label: string }[] = [
  { value: "password", label: "Password" },
  { value: "key-only", label: "SSH key" },
  { value: "disabled", label: "Disabled" },
];

interface StepNetworkProps {
  ssid: string;
  onSsid: (v: string) => void;
  knownNetworks: string[];
  scanning: boolean;
  onRescan: () => void;
  password: string;
  onPassword: (v: string) => void;
  showPassword: boolean;
  onToggleShowPassword: () => void;
  hostname: string;
  onHostname: (v: string) => void;
  controlServer: string;
  onControlServer: (v: string) => void;
  authKey: string;
  onAuthKey: (v: string) => void;
  sshMode: SshMode;
  onSshMode: (v: SshMode) => void;
  sshKey: string;
  onSshKey: (v: string) => void;
  detectedKeys: SshPublicKey[];
  onChooseKeyFile: () => void;
  devicePassword: string;
  onDevicePassword: (v: string) => void;
  onBack: () => void;
  onNext: () => void;
}

export function StepNetwork({
  ssid,
  onSsid,
  knownNetworks,
  scanning,
  onRescan,
  password,
  onPassword,
  showPassword,
  onToggleShowPassword,
  hostname,
  onHostname,
  controlServer,
  onControlServer,
  authKey,
  onAuthKey,
  sshMode,
  onSshMode,
  sshKey,
  onSshKey,
  detectedKeys,
  onChooseKeyFile,
  devicePassword,
  onDevicePassword,
  onBack,
  onNext,
}: StepNetworkProps) {
  const trimmedHostname = hostname.trim();
  const hostnameEmpty = trimmedHostname === "";
  const hostnameValid = isValidHostname(hostname);
  const controlServerValid = isValidControlServer(controlServer);
  const missingAuthKey = needsAuthKey(controlServer, authKey);
  const remoteAccessValid = controlServerValid && !missingAuthKey;
  const sshKeyInvalid = sshMode === "key-only" && isInvalidSshKey(sshKey);
  const devicePasswordWeak =
    sshMode === "password" && isWeakDevicePassword(devicePassword);

  const [remoteOpen, setRemoteOpen] = useState(
    () => controlServer.trim() !== "" || authKey.trim() !== "",
  );
  const remoteExpanded = remoteOpen || !remoteAccessValid;

  const [accessOpen, setAccessOpen] = useState(false);
  // Force it open if there's a validation error, so the operator can see/fix it.
  const accessExpanded = accessOpen || sshKeyInvalid || devicePasswordWeak;

  const [showDevicePassword, setShowDevicePassword] = useState(false);

  const [showControlServer, setShowControlServer] = useState(
    () => controlServer.trim() !== "",
  );

  function useTailscaleInstead() {
    onControlServer("");
    setShowControlServer(false);
  }

  return (
    <StepShell
      heading="Network & access"
      description="These settings are written to the card so the device connects on first boot."
      back={{ onClick: onBack }}
      next={{
        label: "Next",
        onClick: onNext,
        disabled:
          !hostnameValid ||
          !remoteAccessValid ||
          sshKeyInvalid ||
          devicePasswordWeak,
      }}
    >
      <div className="flex max-w-xl flex-col gap-5">
        <div className="flex flex-col gap-2">
          <Label htmlFor="ssid">WiFi network (SSID)</Label>
          <div className="flex items-start gap-2">
            <Combobox
              items={knownNetworks}
              inputValue={ssid}
              onInputValueChange={(value) => onSsid(value)}
            >
              <ComboboxInput
                id="ssid"
                placeholder="Pick a known network or type one"
                className="flex-1"
              />
              <ComboboxContent>
                <ComboboxEmpty>
                  No matching network — it'll be saved as typed.
                </ComboboxEmpty>
                <ComboboxList>
                  {(network: string) => (
                    <ComboboxItem key={network} value={network}>
                      <Wifi className="text-muted-foreground" />
                      {network}
                    </ComboboxItem>
                  )}
                </ComboboxList>
              </ComboboxContent>
            </Combobox>
            <Button
              type="button"
              variant="secondary"
              onClick={onRescan}
              disabled={scanning}
            >
              <RefreshCw className={scanning ? "animate-spin" : undefined} />
              {scanning ? "Scanning…" : "Rescan"}
            </Button>
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <Label htmlFor="password">WiFi password</Label>
          <div className="flex gap-2">
            <Input
              id="password"
              type={showPassword ? "text" : "password"}
              value={password}
              onChange={(e) => onPassword(e.currentTarget.value)}
              className="flex-1"
            />
            <Button
              type="button"
              variant="secondary"
              size="icon"
              onClick={onToggleShowPassword}
              aria-label={showPassword ? "Hide password" : "Show password"}
            >
              {showPassword ? <EyeOff /> : <Eye />}
            </Button>
          </div>
        </div>

        <div className="flex flex-col gap-2">
          <Label htmlFor="hostname">
            Hostname
            <span className="text-destructive" aria-hidden="true">
              *
            </span>
          </Label>
          <Input
            id="hostname"
            value={hostname}
            onChange={(e) => onHostname(e.currentTarget.value)}
            placeholder="e.g. falcon-01"
            required
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            aria-invalid={!hostnameEmpty && !hostnameValid}
            aria-describedby="hostname-help"
          />
          <p id="hostname-help" className="text-sm text-muted-foreground">
            {!hostnameEmpty && !hostnameValid ? (
              <span className="text-destructive">
                Use lowercase letters, numbers, and hyphens only (e.g.
                falcon-01).
              </span>
            ) : (
              <>
                Reachable at{" "}
                <span className="font-medium text-foreground">
                  {trimmedHostname === "" ? "falcon-01" : trimmedHostname}.local
                </span>{" "}
                — pick a name you'll spot in a fleet.
              </>
            )}
          </p>
        </div>

        <div className="flex flex-col gap-2 border-t border-border pt-5">
          <button
            type="button"
            onClick={() => setRemoteOpen((v) => !v)}
            aria-expanded={remoteExpanded}
            className="flex items-center gap-1.5 text-sm font-medium text-foreground"
          >
            Remote access (optional)
            <ChevronDown
              className={`size-4 text-muted-foreground transition-transform ${
                remoteExpanded ? "rotate-180" : ""
              }`}
            />
          </button>
          {remoteExpanded && (
            <div className="flex flex-col gap-4">
              <p className="text-sm text-muted-foreground">
                Reach this drone from anywhere over a private{" "}
                {showControlServer ? "Headscale" : "Tailscale"} network.
              </p>

              {showControlServer && (
                <div className="flex flex-col gap-2">
                  <div className="flex items-baseline justify-between">
                    <Label htmlFor="control-server">
                      Control server (Headscale)
                    </Label>
                    <button
                      type="button"
                      onClick={useTailscaleInstead}
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
                    aria-invalid={!controlServerValid}
                    aria-describedby="control-server-error"
                  />
                  {!controlServerValid && (
                    <p
                      id="control-server-error"
                      className="text-sm text-destructive"
                    >
                      Enter a full URL starting with http:// or https://.
                    </p>
                  )}
                </div>
              )}

              <div className="flex flex-col gap-2">
                <div className="flex items-baseline justify-between">
                  <Label htmlFor="auth-key">Pre-auth key</Label>
                  {!showControlServer && (
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
                    showControlServer
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
                    A control server needs a pre-auth key — without one, nothing
                    is written. Clear the server to use Tailscale instead.
                  </p>
                ) : (
                  <p
                    id="auth-key-help"
                    className="text-sm text-muted-foreground"
                  >
                    The device joins automatically on first boot — no sign-in.
                    The key is written to the card, so use a{" "}
                    <span className="font-medium text-foreground">
                      short-expiry or ephemeral
                    </span>{" "}
                    key. Leave blank to set it up later from the dashboard.
                  </p>
                )}
              </div>

              {!showControlServer && (
                <button
                  type="button"
                  onClick={() => setShowControlServer(true)}
                  className="flex items-center gap-1.5 self-start text-sm text-muted-foreground hover:text-foreground"
                >
                  <Plus className="size-4" />
                  Use a self-hosted control server (Headscale)
                </button>
              )}
            </div>
          )}
        </div>

        <div className="flex flex-col gap-3 border-t border-border pt-5">
          <button
            type="button"
            onClick={() => setAccessOpen((v) => !v)}
            aria-expanded={accessExpanded}
            className="flex items-center gap-1.5 text-sm font-medium text-foreground"
          >
            Device access (optional)
            <ChevronDown
              className={`size-4 text-muted-foreground transition-transform ${
                accessExpanded ? "rotate-180" : ""
              }`}
            />
          </button>
          {accessExpanded && (
            <>
              <p className="text-sm text-muted-foreground">
                Default login is{" "}
                <span className="font-medium text-foreground">pi</span> /{" "}
                <span className="font-medium text-foreground">raspberry</span> —
                set a key or password before deploying.
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
              <p id="ssh-key-help" className="text-sm text-muted-foreground">
                {sshKeyInvalid ? (
                  <span className="text-destructive">
                    That doesn't look like an SSH public key — it should start
                    with ssh-ed25519, ssh-rsa, …
                  </span>
                ) : (
                  <>Password login is off. Leave blank to keep the default.</>
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
              <p
                id="device-password-help"
                className="text-sm text-muted-foreground"
              >
                {devicePasswordWeak ? (
                  <span className="text-destructive">
                    Use at least {MIN_DEVICE_PASSWORD} characters.
                  </span>
                ) : (
                  <>
                    Sets the{" "}
                    <span className="font-medium text-foreground">pi</span>{" "}
                    account password, replacing the default. Leave blank to keep
                    the image default.
                  </>
                )}
              </p>
            </div>
          )}

              {sshMode === "disabled" && (
                <p className="text-sm text-muted-foreground">
                  SSH is turned off. Manage this drone from its web dashboard
                  instead.
                </p>
              )}
            </>
          )}
        </div>
      </div>
    </StepShell>
  );
}
