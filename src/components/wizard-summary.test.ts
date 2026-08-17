import assert from "node:assert/strict";
import { test } from "node:test";

import { buildSummary, type SummaryInput } from "./wizard-summary.ts";
import { defaultHostname } from "./default-hostname.ts";

const base: SummaryInput = {
  image: "Aircast OS v0.3.0",
  hostname: "falcon-01",
  ssid: "field-net",
  authKey: "",
  controlServer: "",
  sshMode: "key-only",
  sshKey: "",
  detectedKeys: [],
  devicePassword: "",
};

const valueOf = (input: SummaryInput, label: string) =>
  buildSummary(input).find((i) => i.label === label)?.value;

test("reports the hostname as the address the device answers on", () => {
  assert.equal(valueOf(base, "Hostname"), "falcon-01.local");
  assert.equal(
    valueOf({ ...base, hostname: "  " }, "Hostname"),
    "Image default",
  );
});

test("says when WiFi is not configured", () => {
  assert.equal(valueOf(base, "WiFi"), "field-net");
  assert.equal(valueOf({ ...base, ssid: "" }, "WiFi"), "Not configured");
});

test("remote access is off without a key, whatever the control server says", () => {
  assert.equal(valueOf(base, "Remote access"), "Off");
  assert.equal(
    valueOf({ ...base, controlServer: "https://hs.example.com" }, "Remote access"),
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
    "Headscale · https://hs.example.com",
  );
});

test("an empty key or password means the image default login survives", () => {
  assert.equal(valueOf(base, "Device access"), "Image default (pi / raspberry)");
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
    valueOf({ ...base, sshMode: "password", devicePassword: "s3cret!" }, "Device access"),
    "Password for pi",
  );
  assert.equal(
    valueOf({ ...base, sshMode: "disabled" }, "Device access"),
    "SSH disabled",
  );
});

test("default hostnames are valid and collide rarely", () => {
  const names = Array.from({ length: 500 }, defaultHostname);
  names.forEach((n: string) => assert.match(n, /^aircast-[0-9a-f]{6}$/));
  assert.equal(new Set(names).size, names.length);
});
