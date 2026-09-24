import { useState } from "react";
import { Cable, Eye, EyeOff, RefreshCw, Wifi } from "lucide-react";

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
  isInvalidWifiPassword,
  isValidHostname,
  needsSsid,
} from "@/components/step-network.validation";
import { GENERATED_HOSTNAME } from "@/components/default-hostname";
import { nextFreeHostname } from "@/components/next-hostname";
import { useLocalTailscale } from "@/lib/use-local-tailscale";

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
  noWifi,
  onNoWifi,
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
  noWifi: boolean;
  onNoWifi: (v: boolean) => void;
  onBack: () => void;
  onNext: () => void;
}) {
  const local = useLocalTailscale();
  const [ssidTouched, setSsidTouched] = useState(false);

  const trimmedHostname = hostname.trim();
  const hostnameEmpty = trimmedHostname === "";
  const hostnameValid = isValidHostname(hostname);
  const hostnameGenerated = GENERATED_HOSTNAME.test(trimmedHostname);
  const ssidMissing = needsSsid(noWifi, ssid);

  const takenNames = (local?.peers ?? [])
    .filter((peer) => peer.sameTailnet)
    .map((peer) => peer.hostName.toLowerCase());
  const nameTaken = takenNames.includes(trimmedHostname.toLowerCase());
  const suggestion = nextFreeHostname(trimmedHostname, takenNames);

  const wifiPasswordInvalid = !noWifi && isInvalidWifiPassword(password);

  return (
    <StepShell
      heading="Network"
      description={
        noWifi
          ? "What this device is called. Its network comes from Ethernet or a modem, so nothing about WiFi is written."
          : "How the device gets online, and what it's called. Written to the card and applied on first boot."
      }
      back={{ onClick: onBack }}
      next={{
        label: "Next",
        onClick: onNext,
        disabled: !hostnameValid || ssidMissing || wifiPasswordInvalid,
      }}
    >
      <div className="flex max-w-xl flex-col gap-5">
        {noWifi ? (
          <div className="flex flex-col gap-2 rounded-xl border border-border p-4">
            <p className="text-sm font-medium">
              No WiFi will be written to the card
            </p>
            <p className="text-sm text-muted-foreground">
              This device gets its network from Ethernet or a cellular modem.
            </p>
            <Button
              type="button"
              variant="secondary"
              className="mt-1 self-start"
              onClick={() => onNoWifi(false)}
            >
              <Wifi className="size-4" />
              Set up WiFi instead
            </Button>
          </div>
        ) : (
          <>
            <div className="flex flex-col gap-2">
              <Label htmlFor="ssid">WiFi network (SSID)</Label>
              <div className="flex items-start gap-2">
                <Combobox
                  items={knownNetworks}
                  inputValue={ssid}
                  onInputValueChange={(value) => {
                    setSsidTouched(true);
                    onSsid(value);
                  }}
                >
                  <ComboboxInput
                    id="ssid"
                    placeholder="Pick a known network or type one"
                    className="flex-1"
                    onBlur={() => setSsidTouched(true)}
                    aria-invalid={ssidMissing && ssidTouched}
                    aria-describedby="ssid-help"
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
              <p
                id="ssid-help"
                className={
                  ssidMissing && ssidTouched
                    ? "text-sm text-destructive"
                    : "text-sm text-muted-foreground"
                }
              >
                {ssidMissing
                  ? "Enter the network the device joins on first boot — without it the device never comes online."
                  : "The device joins this network on first boot."}
              </p>
            </div>

            <div className="flex flex-col gap-2">
              <Label htmlFor="password">WiFi password (optional)</Label>
              <div className="flex gap-2">
                <Input
                  id="password"
                  type={showPassword ? "text" : "password"}
                  value={password}
                  onChange={(e) => onPassword(e.currentTarget.value)}
                  className="flex-1"
                  aria-invalid={wifiPasswordInvalid}
                  aria-describedby="password-help"
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
              <p
                id="password-help"
                className={
                  wifiPasswordInvalid
                    ? "text-sm text-destructive"
                    : "text-sm text-muted-foreground"
                }
              >
                {wifiPasswordInvalid
                  ? "A WiFi password is 8–63 characters — the device cannot join with this one."
                  : "Leave blank for an open network."}
              </p>
            </div>

            <button
              type="button"
              onClick={() => onNoWifi(true)}
              className="flex items-center gap-1.5 self-start text-sm text-muted-foreground hover:text-foreground"
            >
              <Cable className="size-4" />
              This device uses Ethernet or cellular only
            </button>
          </>
        )}

        <div className="flex flex-col gap-2">
          <Label htmlFor="hostname">Hostname</Label>
          <Input
            id="hostname"
            value={hostname}
            onChange={(e) => onHostname(e.currentTarget.value)}
            onFocus={(e) => {
              if (hostnameGenerated) e.currentTarget.select();
            }}
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
            ) : nameTaken ? (
              <span className="text-amber-600 dark:text-amber-400">
                <span className="font-medium">{trimmedHostname}</span> is
                already on your tailnet — Tailscale will name this one{" "}
                <span className="font-medium">{trimmedHostname}-1</span>, so the
                address above won't be the one it answers on.
              </span>
            ) : (
              <>
                Reachable at{" "}
                <span className="font-medium text-foreground">
                  {trimmedHostname}.local
                </span>

              </>
            )}
          </p>
          {nameTaken ? (
            <Button
              type="button"
              variant="secondary"
              size="sm"
              className="self-start"
              onClick={() => onHostname(suggestion)}
            >
              Use {suggestion}
            </Button>
          ) : null}
        </div>

      </div>
    </StepShell>
  );
}
