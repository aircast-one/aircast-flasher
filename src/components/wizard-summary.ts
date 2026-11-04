import type {
  AircastdOutcome,
  Backup,
  KeyWrite,
  SshMode,
  SshPublicKey,
} from "@/types";
import { backupLabel } from "@/lib/format";
import { STEP, type WizardStep } from "@/components/wizard-types";
import type { BackupMarks } from "@/components/restore-card";
import {
  isValidControlServer,
  joinsTailnet,
  keyCardName,
  mintsKeys,
} from "@/components/step-network.validation";

export interface SummaryItem {
  label: string;
  value: string;
  warn?: boolean;
  fromBackup?: boolean;
  step: WizardStep;
}

export interface SummaryInput {
  image: string;
  aircastd: AircastdOutcome | null;
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
  restore: {
    picked: boolean;
    backup: Backup | null;
    sameDrone: boolean | null;
  } | null;
  rejoins: string | null;
  marks: BackupMarks;
}

const IMAGE_DEFAULT_LOGIN = "Image default (pi / raspberry)";

const BUILT_IN = "The version built into the image";

function aircastd(outcome: AircastdOutcome | null): SummaryItem {
  const row = { label: "aircastd", step: STEP.storage };
  if (outcome === null) return { ...row, value: "Checking for the newest…" };
  if (outcome === "builtIn") return { ...row, value: BUILT_IN };
  if ("skipped" in outcome)
    return { ...row, value: `${BUILT_IN} · ${outcome.skipped}` };
  return {
    ...row,
    value: `${outcome.added} · installs on first boot if newer`,
  };
}

function remoteAccess(
  authKey: string,
  controlServer: string,
  keyWrittenTo: KeyWrite | null,
  rejoins: string | null,
  fromBackup: boolean,
): SummaryItem {
  const row = { label: "Remote access", step: STEP.access, fromBackup };
  const server = controlServer.trim();
  if (rejoins !== null)
    return { ...row, value: `Rejoins your tailnet as ${rejoins} · no key needed` };
  if (authKey.trim() === "") return { ...row, value: "Off" };
  if (!isValidControlServer(server))
    return { ...row, value: "Off — control server isn't a URL", warn: true };
  const provider =
    server === ""
      ? "Tailscale"
      : `Headscale · ${server.replace(/^https?:\/\//, "")}`;
  const value = mintsKeys(authKey)
    ? `${provider} · new key per card`
    : provider;
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

function encryption(
  e: SummaryInput["encryption"],
  fromBackup: boolean,
): SummaryItem {
  const row = { label: "Disk encryption", step: STEP.access, fromBackup };
  if (e === null) return { ...row, value: "Off" };
  if (e.tang && e.cloud)
    return { ...row, value: "LUKS2 · your key server + Aircast Cloud" };
  if (e.cloud) return { ...row, value: "LUKS2 · Aircast Cloud" };
  if (e.tang) return { ...row, value: "LUKS2 · your key server" };
  return { ...row, value: "On · no key server yet", warn: true };
}

function fight(input: SummaryInput): SummaryItem {
  const row = {
    label: "Fight mode",
    step: STEP.access,
    fromBackup: input.marks.fight,
  };
  if (!input.fightMode) return { ...row, value: "Off" };
  return input.rejoins !== null ||
    joinsTailnet(input.authKey, input.controlServer)
    ? { ...row, value: "On · no logs, tailnet only" }
    : { ...row, value: "On · needs remote access", warn: true };
}

function restore(r: SummaryInput["restore"]): SummaryItem {
  const row = { label: "Restore", step: STEP.network };
  if (r === null) return { ...row, value: "Off" };
  if (!r.picked) return { ...row, value: "No drone picked yet", warn: true };
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
    aircastd(input.aircastd),
    {
      label: "Hostname",
      value: hostname === "" ? "Image default" : `${hostname}.local`,
      step: STEP.network,
    },
    wifi(input.noWifi, input.ssid.trim(), input.wifilessBoard),
    remoteAccess(
      input.authKey,
      input.controlServer,
      input.keyWrittenTo,
      input.rejoins,
      input.marks.remote,
    ),
    {
      label: "Device access",
      value: access,
      warn: access === IMAGE_DEFAULT_LOGIN,
      fromBackup: input.marks.ssh,
      step: STEP.access,
    },
    encryption(input.encryption, input.marks.encryption),
    fight(input),
    restore(input.restore),
  ];
}
