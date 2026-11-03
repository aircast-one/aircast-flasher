import assert from "node:assert/strict";
import { test } from "node:test";

import {
  NO_MARKS,
  backupMarks,
  cardPreset,
  onlineNow,
  rejoinsTailnet,
} from "./restore-card.ts";
import type { BackupCard, TailnetPeer } from "../types.ts";

const card: BackupCard = {
  fightMode: true,
  passwordLogin: true,
  encryption: {
    threshold: 2,
    tangUrls: ["https://tang.example.com"],
    cloud: true,
  },
  controlServer: "https://hs.example.com",
  tailnetNode: true,
};

const plain: BackupCard = {
  fightMode: false,
  passwordLogin: false,
  encryption: null,
  controlServer: "",
  tailnetNode: false,
};

test("a backup's card presets fight mode, password login, encryption and the server", () => {
  assert.deepEqual(cardPreset(card), {
    fightMode: true,
    remoteAccess: true,
    sshMode: "password",
    encryption: true,
    tangUrls: ["https://tang.example.com"],
    tangThreshold: 2,
    tangCloud: true,
    selfHosted: true,
    controlServer: "https://hs.example.com",
  });
  assert.deepEqual(cardPreset(plain), {
    fightMode: false,
    encryption: false,
    selfHosted: false,
  });
});

test("unknown key servers turn encryption on and leave the servers to the operator", () => {
  assert.deepEqual(
    cardPreset({
      ...plain,
      encryption: { threshold: 0, tangUrls: [], cloud: false },
    }),
    { fightMode: false, encryption: true, selfHosted: false },
  );
});

test("only the same drone, with its node in the backup, rejoins without a key", () => {
  assert.equal(rejoinsTailnet(card, true), true);
  assert.equal(rejoinsTailnet(card, false), false);
  assert.equal(rejoinsTailnet(card, null), false);
  assert.equal(rejoinsTailnet({ ...card, tailnetNode: false }, true), false);
  assert.equal(rejoinsTailnet(null, true), false);
});

test("a setting is marked from the backup while it still matches the backup", () => {
  const now = {
    rejoins: false,
    selfHosted: true,
    controlServer: "https://hs.example.com",
    encryption: true,
    fightMode: true,
    sshMode: "password" as const,
  };
  assert.deepEqual(backupMarks(card, now), {
    remote: true,
    encryption: true,
    fight: true,
    ssh: true,
  });
  assert.deepEqual(
    backupMarks(card, {
      ...now,
      selfHosted: false,
      encryption: false,
      fightMode: false,
      sshMode: "key-only",
    }),
    NO_MARKS,
  );
  assert.equal(
    backupMarks(card, { ...now, selfHosted: false, rejoins: true }).remote,
    true,
  );
  assert.deepEqual(backupMarks(null, now), NO_MARKS);
});

test("a drone counts as online only when a peer by that name is online", () => {
  const peer = (hostName: string, online: boolean): TailnetPeer => ({
    hostName,
    dnsName: `${hostName}.tail9c2f.ts.net`,
    os: "linux",
    online,
    ip: "100.64.0.7",
    sameTailnet: true,
  });
  assert.equal(onlineNow("falcon", [peer("Falcon", true)]), true);
  assert.equal(onlineNow("falcon", [peer("falcon", false)]), false);
  assert.equal(onlineNow("falcon", [peer("falcon-02", true)]), false);
});
