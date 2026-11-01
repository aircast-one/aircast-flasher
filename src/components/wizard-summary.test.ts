import assert from "node:assert/strict";
import { test } from "node:test";

import { buildSummary, type SummaryInput } from "./wizard-summary.ts";
import { defaultHostname } from "./default-hostname.ts";

const base: SummaryInput = {
  image: "Aircast OS v0.3.0",
  hostname: "falcon-01",
  ssid: "field-net",
  noWifi: false,
  wifilessBoard: null,
  authKey: "",
  keyWrittenTo: null,
  controlServer: "",
  sshMode: "key-only",
  sshKey: "",
  detectedKeys: [],
  devicePassword: "",
  encryption: null,
  fightMode: false,
  restore: null,
};

const rowOf = (input: SummaryInput, label: string) =>
  buildSummary(input).find((i) => i.label === label);

const valueOf = (input: SummaryInput, label: string) =>
  rowOf(input, label)?.value;

test("reports the hostname as the address the device answers on", () => {
  assert.equal(valueOf(base, "Hostname"), "falcon-01.local");
  assert.equal(
    valueOf({ ...base, hostname: "  " }, "Hostname"),
    "Image default",
  );
});

test("says when WiFi is not configured, and flags it", () => {
  assert.equal(valueOf(base, "WiFi"), "field-net");
  assert.equal(rowOf(base, "WiFi")?.warn, undefined);
  assert.equal(valueOf({ ...base, ssid: "" }, "WiFi"), "Not configured");
  assert.equal(rowOf({ ...base, ssid: "" }, "WiFi")?.warn, true);
});

test("skipping WiFi on purpose is stated, not flagged", () => {
  const ethernet = { ...base, ssid: "", noWifi: true };
  assert.equal(valueOf(ethernet, "WiFi"), "Ethernet or cellular only");
  assert.equal(rowOf(ethernet, "WiFi")?.warn, undefined);
});

test("WiFi on a board without it is flagged with the adapter it needs", () => {
  const opi = { ...base, wifilessBoard: "Orange Pi 5" };
  assert.equal(
    valueOf(opi, "WiFi"),
    "field-net — Orange Pi 5 needs a USB WiFi adapter",
  );
  assert.equal(rowOf(opi, "WiFi")?.warn, true);
  assert.equal(
    valueOf({ ...opi, noWifi: true }, "WiFi"),
    "Ethernet or cellular only",
  );
});

test("the surviving image default login is flagged", () => {
  assert.equal(rowOf(base, "Device access")?.warn, true);
  assert.equal(
    rowOf({ ...base, sshKey: "ssh-rsa AAAAB3Nz" }, "Device access")?.warn,
    false,
  );
});

test("remote access is off without a key, whatever the control server says", () => {
  assert.equal(valueOf(base, "Remote access"), "Off");
  assert.equal(
    valueOf(
      { ...base, controlServer: "https://hs.example.com" },
      "Remote access",
    ),
    "Off",
  );
});

test("distinguishes Tailscale from a self-hosted control server", () => {
  const withKey = { ...base, authKey: "tskey-auth-abc" };
  assert.equal(valueOf(withKey, "Remote access"), "Tailscale");
  assert.equal(
    valueOf(
      { ...withKey, controlServer: "https://hs.example.com" },
      "Remote access",
    ),
    "Headscale · hs.example.com",
  );
});

test("a control server that isn't a URL means remote access is skipped", () => {
  const bad = {
    ...base,
    authKey: "tskey-auth-abc",
    controlServer: "headscale",
  };
  assert.equal(
    valueOf(bad, "Remote access"),
    "Off — control server isn't a URL",
  );
  assert.equal(rowOf(bad, "Remote access")?.warn, true);
});

test("flags a key that already went onto another card", () => {
  const reused = {
    ...base,
    authKey: "hskey-auth-abc",
    controlServer: "https://hs.example.com",
    keyWrittenTo: { hostname: "pizero2", at: "2026-11-01T00:24:00.000Z" },
  };
  assert.equal(
    valueOf(reused, "Remote access"),
    "Headscale · hs.example.com · key already used on pizero2",
  );
  assert.equal(rowOf(reused, "Remote access")?.warn, true);
  const unnamed = { ...reused.keyWrittenTo, hostname: "" };
  assert.equal(
    valueOf({ ...reused, keyWrittenTo: unnamed }, "Remote access"),
    "Headscale · hs.example.com · key already used on a card",
  );
  assert.equal(valueOf({ ...reused, authKey: "" }, "Remote access"), "Off");
});

test("an empty key or password means the image default login survives", () => {
  assert.equal(
    valueOf(base, "Device access"),
    "Image default (pi / raspberry)",
  );
  assert.equal(
    valueOf({ ...base, sshMode: "password" }, "Device access"),
    "Image default (pi / raspberry)",
  );
});

test("names the key that will be written", () => {
  const key = "ssh-ed25519 AAAAC3Nz pavliha@mac";
  assert.equal(
    valueOf(
      {
        ...base,
        sshKey: key,
        detectedKeys: [{ label: "id_ed25519.pub", contents: key }],
      },
      "Device access",
    ),
    "SSH key · id_ed25519.pub",
  );
  assert.equal(
    valueOf({ ...base, sshKey: key }, "Device access"),
    "SSH key · pavliha@mac",
  );
  assert.equal(
    valueOf({ ...base, sshKey: "ssh-rsa AAAAB3Nz" }, "Device access"),
    "SSH key · pasted",
  );
});

test("reports a set password and a disabled SSH server", () => {
  assert.equal(
    valueOf(
      { ...base, sshMode: "password", devicePassword: "s3cret!" },
      "Device access",
    ),
    "Password for pi",
  );
  assert.equal(
    valueOf({ ...base, sshMode: "disabled" }, "Device access"),
    "SSH disabled",
  );
});

test("the default hostname is the lowest number not already in use", () => {
  assert.equal(defaultHostname(), "aircast-01");
  assert.equal(defaultHostname(["aircast-01"]), "aircast-02");
  assert.equal(defaultHostname(["aircast-01", "aircast-02"]), "aircast-03");
  assert.equal(defaultHostname(["AIRCAST-01", "aircast-03"]), "aircast-02");
  assert.equal(defaultHostname(["falcon", "owel-3"]), "aircast-01");
});

test("reports whether the data partition is encrypted", () => {
  assert.equal(valueOf(base, "Disk encryption"), "Off");
  assert.equal(
    valueOf({ ...base, encryption: { tang: true, cloud: false } }, "Disk encryption"),
    "LUKS2 · your key server",
  );
  assert.equal(
    valueOf({ ...base, encryption: { tang: false, cloud: true } }, "Disk encryption"),
    "LUKS2 · Aircast Cloud",
  );
  assert.equal(
    valueOf({ ...base, encryption: { tang: true, cloud: true } }, "Disk encryption"),
    "LUKS2 · your key server + Aircast Cloud",
  );
});

test("encryption with no usable key server is flagged", () => {
  const row = buildSummary({
    ...base,
    encryption: { tang: false, cloud: false },
  }).find((r) => r.label === "Disk encryption");
  assert.equal(row?.value, "On · no key server yet");
  assert.equal(row?.warn, true);
});

test("fight mode is stated either way", () => {
  assert.equal(valueOf(base, "Fight mode"), "Off");
  assert.equal(
    valueOf({ ...base, fightMode: true, authKey: "tskey-auth-abc" }, "Fight mode"),
    "On · no logs, tailnet only",
  );
});

test("fight mode without a tailnet is flagged as unreachable", () => {
  const stranded = { ...base, fightMode: true };
  assert.equal(valueOf(stranded, "Fight mode"), "On · needs remote access");
  assert.equal(rowOf(stranded, "Fight mode")?.warn, true);
  const badServer = {
    ...stranded,
    authKey: "hskey-auth-abc",
    controlServer: "hs.example.com",
  };
  assert.equal(rowOf(badServer, "Fight mode")?.warn, true);
});

test("a restore names the backup and which drone it is for", () => {
  const backup = {
    id: "falcon-20261101120000",
    hostname: "falcon",
    created: "2026-11-01T12:00:00Z",
  };
  assert.equal(valueOf(base, "Restore"), "Off");
  const same = { ...base, restore: { backup, sameDrone: true } };
  assert.match(valueOf(same, "Restore") ?? "", /^falcon · .+ · same drone$/);
  assert.equal(rowOf(same, "Restore")?.warn, undefined);
  assert.match(
    valueOf({ ...base, restore: { backup, sameDrone: false } }, "Restore") ??
      "",
    /· new drone$/,
  );
  assert.equal(
    rowOf({ ...base, restore: { backup, sameDrone: null } }, "Restore")?.warn,
    true,
  );
  assert.equal(
    valueOf({ ...base, restore: { backup: null, sameDrone: true } }, "Restore"),
    "Backup no longer on this computer",
  );
});
