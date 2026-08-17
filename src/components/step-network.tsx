import { useState } from "react";
import { Eye, EyeOff, RefreshCw, Wifi } from "lucide-react";

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
import { CollapsibleSection } from "@/components/collapsible-section";
import {
  RemoteAccessSection,
  type RemoteAccessProps,
} from "@/components/remote-access-section";
import {
  DeviceAccessSection,
  type DeviceAccessProps,
} from "@/components/device-access-section";
import {
  isInvalidSshKey,
  isValidControlServer,
  isValidHostname,
  isWeakDevicePassword,
  needsAuthKey,
  needsControlServer,
  needsSsid,
} from "@/components/step-network.validation";

type RemoteAccessInputs = Pick<
  RemoteAccessProps,
  "controlServer" | "onControlServer" | "authKey" | "onAuthKey"
>;

type DeviceAccessInputs = Pick<
  DeviceAccessProps,
  | "sshMode"
  | "onSshMode"
  | "sshKey"
  | "onSshKey"
  | "detectedKeys"
  | "onChooseKeyFile"
  | "devicePassword"
  | "onDevicePassword"
>;

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
  remote,
  access,
  onBack,
  onNext,
}: {
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
  remote: RemoteAccessInputs;
  access: DeviceAccessInputs;
  onBack: () => void;
  onNext: () => void;
}) {
  const [selfHosted, setSelfHosted] = useState(
    () => remote.controlServer.trim() !== "",
  );

  const trimmedHostname = hostname.trim();
  const hostnameEmpty = trimmedHostname === "";
  const hostnameValid = isValidHostname(hostname);
  const ssidMissing = needsSsid(ssid, password);

  const controlServerValid = isValidControlServer(remote.controlServer);
  const controlServerMissing = needsControlServer(
    selfHosted,
    remote.controlServer,
  );
  const missingAuthKey = needsAuthKey(remote.controlServer, remote.authKey);
  const remoteAccessValid =
    controlServerValid && !controlServerMissing && !missingAuthKey;

  const sshKeyInvalid =
    access.sshMode === "key-only" && isInvalidSshKey(access.sshKey);
  const devicePasswordWeak =
    access.sshMode === "password" &&
    isWeakDevicePassword(access.devicePassword);

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
          ssidMissing ||
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
                aria-invalid={ssidMissing}
                aria-describedby="ssid-error"
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
          {ssidMissing && (
            <p id="ssid-error" className="text-sm text-destructive">
              Add the network name — a password on its own isn't written to the
              card.
            </p>
          )}
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
            aria-invalid={!hostnameValid}
            aria-describedby="hostname-help"
          />
          <p id="hostname-help" className="text-sm text-muted-foreground">
            {hostnameEmpty ? (
              <span className="text-destructive">
                Enter a hostname to continue.
              </span>
            ) : !hostnameValid ? (
              <span className="text-destructive">
                Use lowercase letters, numbers, and hyphens only (e.g.
                falcon-01).
              </span>
            ) : (
              <>
                Reachable at{" "}
                <span className="font-medium text-foreground">
                  {trimmedHostname}.local
                </span>{" "}
                — pick a name you'll spot in a fleet.
              </>
            )}
          </p>
        </div>

        <CollapsibleSection
          title="Remote access (optional)"
          forceOpen={!remoteAccessValid}
          initiallyOpen={
            remote.controlServer.trim() !== "" || remote.authKey.trim() !== ""
          }
        >
          <RemoteAccessSection
            {...remote}
            selfHosted={selfHosted}
            onSelfHosted={setSelfHosted}
            controlServerValid={controlServerValid}
            controlServerMissing={controlServerMissing}
            missingAuthKey={missingAuthKey}
          />
        </CollapsibleSection>

        <CollapsibleSection
          title="Device access (optional)"
          forceOpen={sshKeyInvalid || devicePasswordWeak}
          initiallyOpen={false}
        >
          <DeviceAccessSection
            {...access}
            sshKeyInvalid={sshKeyInvalid}
            devicePasswordWeak={devicePasswordWeak}
          />
        </CollapsibleSection>
      </div>
    </StepShell>
  );
}
