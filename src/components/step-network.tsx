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

const HOSTNAME_PATTERN = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;

export function isValidHostname(value: string): boolean {
  return HOSTNAME_PATTERN.test(value.trim());
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
  onBack: () => void;
  onNext: () => void;
}) {
  const trimmedHostname = hostname.trim();
  const hostnameEmpty = trimmedHostname === "";
  const hostnameValid = isValidHostname(hostname);

  return (
    <StepShell
      heading="WiFi & hostname"
      description="These settings are written to the card so the device connects on first boot."
      back={{ onClick: onBack }}
      next={{ label: "Next", onClick: onNext, disabled: !hostnameValid }}
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
                Use lowercase letters, numbers, and hyphens only (e.g. falcon-01).
              </span>
            ) : (
              <>
                This becomes your drone's name on the network — you'll reach it at{" "}
                <span className="font-medium text-foreground">
                  {trimmedHostname === "" ? "falcon-01" : trimmedHostname}.local
                </span>
                . Pick something you'll recognise in a fleet.
              </>
            )}
          </p>
        </div>
      </div>
    </StepShell>
  );
}
