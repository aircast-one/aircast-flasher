import assert from "node:assert/strict";
import { test } from "node:test";

import { buildSummary, type SummaryInput } from "./wizard-summary.ts";
import { defaultHostname } from "./default-hostname.ts";

const base: SummaryInput = {
  image: "Aircast OS v0.3.0",
  hostname: "falcon-01",
  ssid: "field-net",
  noWifi: false,
  authKey: "",
  controlServer: "",
  sshMode: "key-only",
  sshKey: "",
  detectedKeys: [],
  devicePassword: "",
  encryption: null,
  fightMode: false,
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
