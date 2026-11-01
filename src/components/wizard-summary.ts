import type { Backup, KeyWrite, SshMode, SshPublicKey } from "@/types";
import { backupLabel } from "@/lib/format";
import { STEP, type WizardStep } from "@/components/wizard-types";
import {
  isValidControlServer,
  joinsTailnet,
  keyCardName,
} from "@/components/step-network.validation";

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
  wifilessBoard: string | null;
  authKey: string;
  keyWrittenTo: KeyWrite | null;
  controlServer: string;
  sshMode: SshMode;
  sshKey: string;
  detectedKeys: SshPublicKey[];
  devicePassword: string;
  encryption: { tang: boolean; cloud: boolean } | null;
  fightMode: boolean;
  restore: { backup: Backup | null; sameDrone: boolean | null } | null;
}

const IMAGE_DEFAULT_LOGIN = "Image default (pi / raspberry)";

function remoteAccess(
  authKey: string,
  controlServer: string,
  keyWrittenTo: KeyWrite | null,
): SummaryItem {
  const row = { label: "Remote access", step: STEP.access };
  const server = controlServer.trim();
  if (authKey.trim() === "") return { ...row, value: "Off" };
  if (!isValidControlServer(server))
    return { ...row, value: "Off — control server isn't a URL", warn: true };
  const value =
    server === ""
      ? "Tailscale"
      : `Headscale · ${server.replace(/^https?:\/\//, "")}`;
  return keyWrittenTo === null
    ? { ...row, value }
    : {
        ...row,
        value: `${value} · key already used on ${keyCardName(keyWrittenTo)}`,
        warn: true,
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

function wifi(
  noWifi: boolean,
  ssid: string,
  wifilessBoard: string | null,
): SummaryItem {
  const row = { label: "WiFi", step: STEP.network };
  if (noWifi) return { ...row, value: "Ethernet or cellular only" };
  return ssid === ""
    ? { ...row, value: "Not configured", warn: true }
    : wifilessBoard !== null
      ? {
          ...row,
          value: `${ssid} — ${wifilessBoard} needs a USB WiFi adapter`,
          warn: true,
        }
      : { ...row, value: ssid };
}

function encryption(e: SummaryInput["encryption"]): SummaryItem {
  const row = { label: "Disk encryption", step: STEP.access };
  if (e === null) return { ...row, value: "Off" };
  if (e.tang && e.cloud)
    return { ...row, value: "LUKS2 · your key server + Aircast Cloud" };
  if (e.cloud) return { ...row, value: "LUKS2 · Aircast Cloud" };
  if (e.tang) return { ...row, value: "LUKS2 · your key server" };
  return { ...row, value: "On · no key server yet", warn: true };
}

function fight(input: SummaryInput): SummaryItem {
  const row = { label: "Fight mode", step: STEP.access };
  if (!input.fightMode) return { ...row, value: "Off" };
  return joinsTailnet(input.authKey, input.controlServer)
    ? { ...row, value: "On · no logs, tailnet only" }
    : { ...row, value: "On · needs remote access", warn: true };
}

function restore(r: SummaryInput["restore"]): SummaryItem {
  const row = { label: "Restore", step: STEP.network };
  if (r === null) return { ...row, value: "Off" };
  if (r.backup === null)
    return { ...row, value: "Backup no longer on this computer", warn: true };
  if (r.sameDrone === null)
    return {
      ...row,
      value: `${backupLabel(r.backup)} · same or new drone not chosen`,
      warn: true,
    };
  return {
    ...row,
    value: `${backupLabel(r.backup)} · ${r.sameDrone ? "same drone" : "new drone"}`,
  };
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
    wifi(input.noWifi, input.ssid.trim(), input.wifilessBoard),
    remoteAccess(input.authKey, input.controlServer, input.keyWrittenTo),
    {
      label: "Device access",
      value: access,
      warn: access === IMAGE_DEFAULT_LOGIN,
      step: STEP.access,
    },
    encryption(input.encryption),
    fight(input),
    restore(input.restore),
  ];
}
