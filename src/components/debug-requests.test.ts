import { mock, test } from "node:test";
import assert from "node:assert/strict";

import { runDebugRequest, type DebugControls, type DebugView } from "./debug-requests.ts";

const view: DebugView = {
  page: "storage",
  boards: ["rpi", "orangepi-5"],
  board: "rpi",
  sourceKind: "aircast",
  versions: ["v0.4.0-beta.4", "v0.3.5"],
  version: "v0.3.5",
  hostname: "aircast-01",
  ssid: "",
  noWifi: true,
  wifiPasswordSet: false,
  remoteAccess: false,
  authKeySet: false,
  sshMode: "key-only",
  devicePasswordSet: false,
  encryption: false,
  fightMode: false,
  disks: [{ path: "\\\\.\\PhysicalDrive1", name: "USB card", size: 8_000_000_000 }],
  disk: "\\\\.\\PhysicalDrive1",
  problems: [],
  canFlash: true,
  status: "Ready to write",
  progress: { phase: "idle" },
  writing: false,
  written: false,
  error: null,
};

const controls = (overrides: Partial<DebugView> = {}) => ({
  view: { ...view, ...overrides },
  consent: mock.fn((_share: boolean) => undefined),
  goto: mock.fn((_page: string) => undefined),
  board: mock.fn((_id: string) => undefined),
  version: mock.fn((_version: string) => undefined),
  disk: mock.fn((_path: string) => undefined),
  text: mock.fn((_field: string, _value: string) => undefined),
  toggle: mock.fn((_field: string, _on: boolean) => undefined),
  sshMode: mock.fn((_mode: string) => undefined),
  flash: mock.fn(() => undefined),
  cancel: mock.fn(() => undefined),
}) satisfies DebugControls;

test("state reports the wizard and never a secret", () => {
  const reply = runDebugRequest({ action: "state" }, controls());
  assert.equal(reply.ok, true);
  assert.equal(reply.page, "storage");
  const text = JSON.stringify(reply);
  ["wifiPassword\"", "authKey\"", "devicePassword\""].forEach((secret) =>
    assert.ok(!text.includes(secret), `state carried ${secret}`),
  );
});

test("set applies every setting only when all of them are valid", () => {
  const good = controls();
  const reply = runDebugRequest(
    { action: "set", board: "orangepi-5", version: "v0.4.0-beta.4", hostname: "falcon-02", noWifi: false },
    good,
  );
  assert.deepEqual(reply, { ok: true, applied: ["board", "version", "hostname", "noWifi"] });
  assert.deepEqual(good.board.mock.calls.map((c) => c.arguments), [["orangepi-5"]]);
  assert.deepEqual(good.version.mock.calls.map((c) => c.arguments), [["v0.4.0-beta.4"]]);
  assert.deepEqual(good.text.mock.calls.map((c) => c.arguments), [["hostname", "falcon-02"]]);
  assert.deepEqual(good.toggle.mock.calls.map((c) => c.arguments), [["noWifi", false]]);

  const bad = controls();
  const refused = runDebugRequest({ action: "set", hostname: "ok", version: "v9", noWifi: "yes", color: "red" }, bad);
  assert.equal(refused.ok, false);
  assert.match(String(refused.reason), /version must be one of: v0.4.0-beta.4, v0.3.5/);
  assert.match(String(refused.reason), /noWifi must be true or false/);
  assert.match(String(refused.reason), /unknown setting color/);
  assert.equal(bad.text.mock.callCount(), 0, "a valid setting beside a bad one is not applied");
});

test("only an offered card can be picked", () => {
  const picked = controls();
  assert.equal(runDebugRequest({ action: "set", disk: "\\\\.\\PhysicalDrive0" }, picked).ok, false);
  assert.equal(runDebugRequest({ action: "set", disk: "\\\\.\\PhysicalDrive1" }, picked).ok, true);
  assert.deepEqual(picked.disk.mock.calls.map((c) => c.arguments), [["\\\\.\\PhysicalDrive1"]]);
});

test("flash refuses with the problems the Storage page shows", () => {
  const blocked = controls({ canFlash: false, problems: ["Insert an SD card"] });
  assert.deepEqual(runDebugRequest({ action: "flash" }, blocked), {
    ok: false,
    reason: "Not ready to write",
    problems: ["Insert an SD card"],
  });
  assert.equal(blocked.flash.mock.callCount(), 0);
  const ready = controls();
  assert.deepEqual(runDebugRequest({ action: "flash" }, ready), { ok: true });
  assert.equal(ready.flash.mock.callCount(), 1);
});

test("pages can't change mid-write and cancel needs a write", () => {
  const writing = controls({ writing: true });
  assert.equal(runDebugRequest({ action: "goto", page: "os" }, writing).ok, false);
  assert.equal(runDebugRequest({ action: "cancel" }, writing).ok, true);
  assert.equal(writing.cancel.mock.callCount(), 1);
  const idle = controls();
  assert.equal(runDebugRequest({ action: "cancel" }, idle).ok, false);
  assert.equal(runDebugRequest({ action: "goto", page: "write" }, idle).ok, false);
  assert.equal(runDebugRequest({ action: "goto", page: "devices" }, idle).ok, true);
  assert.deepEqual(idle.goto.mock.calls.map((c) => c.arguments), [["devices"]]);
});

test("consent and unknown actions answer plainly", () => {
  const asked = controls({ page: "consent" });
  assert.equal(runDebugRequest({ action: "consent" }, asked).ok, false);
  assert.equal(runDebugRequest({ action: "consent", share: false }, asked).ok, true);
  assert.deepEqual(asked.consent.mock.calls.map((c) => c.arguments), [[false]]);
  assert.match(String(runDebugRequest({ action: "dance" }, asked).reason), /state, consent, goto, set, flash, cancel/);
});
