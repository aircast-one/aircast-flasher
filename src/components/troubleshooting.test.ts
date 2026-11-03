import assert from "node:assert/strict";
import { test } from "node:test";

import { troubleshootingTips } from "./troubleshooting.ts";
import type { Board } from "../types.ts";

const pi: Board = {
  id: "rpi",
  name: "Raspberry Pi",
  maker: "Raspberry Pi",
  detail: "",
  builtinWifi: true,
  beta: false,
  tested: true,
};
const base = {
  board: pi,
  usb: false,
  noWifi: false,
  fightMode: false,
  restore: null,
};

test("one list holds every reason a drone may not show up", () => {
  assert.deepEqual(troubleshootingTips(base), [
    "This card holds the Raspberry Pi image. It won't start on a different board.",
  ]);
  const fight = troubleshootingTips({
    ...base,
    fightMode: true,
    board: { ...pi, name: "Orange Pi 5", builtinWifi: false },
  });
  assert.equal(fight.length, 3);
  assert.match(fight[0], /pre-auth key was probably expired/);
  assert.match(fight[1], /Orange Pi 5 has no built-in WiFi/);
});

test("a restored node gets the rejoin advice, not the key advice", () => {
  const tips = troubleshootingTips({
    ...base,
    fightMode: true,
    restore: { host: "falcon", rejoins: true },
  });
  assert.match(tips[0], /rejoins as falcon with the identity from the backup/);
  assert.equal(tips.some((tip) => tip.includes("pre-auth key")), false);
});

test("a USB write or a local image drops the board tips", () => {
  assert.deepEqual(troubleshootingTips({ ...base, usb: true }), []);
  assert.deepEqual(troubleshootingTips({ ...base, board: null }), []);
});
