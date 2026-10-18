import assert from "node:assert/strict";
import { test } from "node:test";

import {
  fightModeUnreachable,
  isInvalidWifiPassword,
  needsControlServer,
} from "./step-network.validation.ts";

test("an open network needs no password", () => {
  assert.equal(isInvalidWifiPassword(""), false);
});

test("rejects passphrases WPA cannot hold", () => {
  assert.equal(isInvalidWifiPassword("short"), true);
  assert.equal(isInvalidWifiPassword("z".repeat(64)), true);
});

test("accepts a real passphrase and a raw 64-hex key", () => {
  assert.equal(isInvalidWifiPassword("hunter22"), false);
  assert.equal(isInvalidWifiPassword("a".repeat(63)), false);
  assert.equal(isInvalidWifiPassword("0123456789abcdef".repeat(4)), false);
});

test("a blank control server is only a problem once a key exists", () => {
  assert.equal(needsControlServer(true, "", ""), false);
  assert.equal(needsControlServer(true, "", "hskey-auth-abc"), true);
  assert.equal(
    needsControlServer(true, "https://hs.example.com", "hskey-auth-abc"),
    false,
  );
  assert.equal(needsControlServer(false, "", "hskey-auth-abc"), false);
});

test("fight mode is unreachable until the drone joins a tailnet", () => {
  assert.equal(fightModeUnreachable(false, "", ""), false);
  assert.equal(fightModeUnreachable(true, "", ""), true);
  assert.equal(fightModeUnreachable(true, "  ", ""), true);
  assert.equal(fightModeUnreachable(true, "hskey-auth-abc", "hs.example.com"), true);
  assert.equal(fightModeUnreachable(true, "tskey-auth-abc", ""), false);
  assert.equal(
    fightModeUnreachable(true, "hskey-auth-abc", "https://hs.example.com"),
    false,
  );
});
