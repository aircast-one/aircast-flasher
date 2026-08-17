import type { SshMode, SshPublicKey } from "@/types";

export interface SummaryItem {
  label: string;
  value: string;
}

export interface SummaryInput {
  image: string;
  hostname: string;
  ssid: string;
  authKey: string;
  controlServer: string;
  sshMode: SshMode;
  sshKey: string;
  detectedKeys: SshPublicKey[];
  devicePassword: string;
}

const IMAGE_DEFAULT_LOGIN = "Image default (pi / raspberry)";

function remoteAccess(authKey: string, controlServer: string): string {
  if (authKey.trim() === "") return "Off";
  const server = controlServer.trim();
  return server === "" ? "Tailscale" : `Headscale · ${server}`;
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

export function buildSummary(input: SummaryInput): SummaryItem[] {
  const hostname = input.hostname.trim();
  const ssid = input.ssid.trim();
  return [
    { label: "Image", value: input.image },
    {
      label: "Hostname",
      value: hostname === "" ? "Image default" : `${hostname}.local`,
    },
    { label: "WiFi", value: ssid === "" ? "Not configured" : ssid },
    {
      label: "Remote access",
      value: remoteAccess(input.authKey, input.controlServer),
    },
    {
      label: "Device access",
      value: deviceAccess(
        input.sshMode,
        input.sshKey,
        input.detectedKeys,
        input.devicePassword,
      ),
    },
  ];
}
