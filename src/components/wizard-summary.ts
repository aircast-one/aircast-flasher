import type { SshMode, SshPublicKey } from "@/types";
import { STEP, type WizardStep } from "@/components/wizard-types";
import { isValidControlServer } from "@/components/step-network.validation";

export interface SummaryItem {
  label: string;
  value: string;
  warn?: boolean;
  step: WizardStep;
}

export interface SummaryInput {
  image: string;
  hostname: string;
  ssid: string;
  noWifi: boolean;
  authKey: string;
  controlServer: string;
  sshMode: SshMode;
  sshKey: string;
  detectedKeys: SshPublicKey[];
  devicePassword: string;
}

const IMAGE_DEFAULT_LOGIN = "Image default (pi / raspberry)";

function remoteAccess(authKey: string, controlServer: string): SummaryItem {
  const row = { label: "Remote access", step: STEP.access };
  const server = controlServer.trim();
  if (authKey.trim() === "") return { ...row, value: "Off" };
  if (server === "") return { ...row, value: "Tailscale" };
  if (!isValidControlServer(server))
    return { ...row, value: "Off — control server isn't a URL", warn: true };
  return {
    ...row,
    value: `Headscale · ${server.replace(/^https?:\/\//, "")}`,
  };
}

function keyName(sshKey: string, detectedKeys: SshPublicKey[]): string {
  const key = sshKey.trim();
  const known = detectedKeys.find((k) => k.contents.trim() === key);
  if (known) return `SSH key · ${known.label}`;
  const comment = key.split(/\s+/)[2];
  return comment === undefined ? "SSH key · pasted" : `SSH key · ${comment}`;
}

function deviceAccess(
  sshMode: SshMode,
  sshKey: string,
  detectedKeys: SshPublicKey[],
  devicePassword: string,
): string {
  if (sshMode === "disabled") return "SSH disabled";
  if (sshMode === "key-only") {
    return sshKey.trim() === ""
      ? IMAGE_DEFAULT_LOGIN
      : keyName(sshKey, detectedKeys);
  }
  return devicePassword === "" ? IMAGE_DEFAULT_LOGIN : "Password for pi";
}

function wifi(noWifi: boolean, ssid: string): SummaryItem {
  const row = { label: "WiFi", step: STEP.network };
  if (noWifi) return { ...row, value: "Ethernet or cellular only" };
  return ssid === ""
    ? { ...row, value: "Not configured", warn: true }
    : { ...row, value: ssid };
}

export function buildSummary(input: SummaryInput): SummaryItem[] {
  const hostname = input.hostname.trim();
  const access = deviceAccess(
    input.sshMode,
    input.sshKey,
    input.detectedKeys,
    input.devicePassword,
  );
  return [
    { label: "Image", value: input.image, step: STEP.os },
    {
      label: "Hostname",
      value: hostname === "" ? "Image default" : `${hostname}.local`,
      step: STEP.network,
    },
    wifi(input.noWifi, input.ssid.trim()),
    remoteAccess(input.authKey, input.controlServer),
    {
      label: "Device access",
      value: access,
      warn: access === IMAGE_DEFAULT_LOGIN,
      step: STEP.access,
    },
  ];
}
