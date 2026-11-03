import { test } from "node:test";
import assert from "node:assert/strict";

import {
  cardLabel,
  cardState,
  cloudLinkSummary,
  linkedAccount,
  retireCandidates,
  waitForSignIn,
  withCloudServer,
  type CloudLink,
} from "./cloud.ts";
import type { CloudCard, CloudSignInStatus } from "../types.ts";

const card = (over: Partial<CloudCard>): CloudCard => ({
  id: "a",
  label: "falcon-01",
  createdAt: "2026-10-17T00:00:00Z",
  lostAt: null,
  lastUnlockAt: null,
  lastUnlockIp: null,
  ...over,
});

test("a lost card reads as lost even if it unlocked before", () => {
  assert.equal(
    cardState(card({ lostAt: "2026-10-17T02:00:00Z", lastUnlockAt: "2026-10-17T01:00:00Z" })),
    "lost",
  );
});

test("a card that unlocked reads as unlocked", () => {
  assert.equal(cardState(card({ lastUnlockAt: "2026-10-17T01:00:00Z" })), "unlocked");
});

test("a fresh card has never unlocked", () => {
  assert.equal(cardState(card({})), "never-unlocked");
});

test("sign-in waits through pending polls and returns the email", async () => {
  const replies: CloudSignInStatus[] = [
    { status: "pending" },
    { status: "pending" },
    { status: "signedIn", email: "op@unit.example" },
  ];
  const polled: string[] = [];
  const email = await waitForSignIn(
    "tok",
    (token) => {
      polled.push(token);
      return Promise.resolve(replies[polled.length - 1]);
    },
    () => Promise.resolve(),
  );
  assert.equal(email, "op@unit.example");
  assert.equal(polled.length, 3);
});

test("sign-in stops with the poll's error, e.g. an expired sign-in", async () => {
  await assert.rejects(
    waitForSignIn(
      "tok",
      () => Promise.reject("The sign-in expired. Start it again."),
      () => Promise.resolve(),
    ),
    /expired/,
  );
});

test("the cloud key server is added to the card's servers with its advertisement", () => {
  const out = withCloudServer(
    { tangServers: [{ url: "http://unit:7500", adv: "{}" }], threshold: 1, recoveryKey: "k" },
    { id: "ks", url: "https://api.aircast.one/v1/tang/ks", advertisement: { payload: "p" } },
  );
  assert.deepEqual(out.tangServers, [
    { url: "http://unit:7500", adv: "{}" },
    { url: "https://api.aircast.one/v1/tang/ks", adv: '{"payload":"p"}' },
  ]);
  assert.equal(out.threshold, 1);
  assert.equal(out.recoveryKey, "k");
});

test("a card with no hostname still gets a label", () => {
  assert.equal(cardLabel("  "), "unnamed card");
  assert.equal(cardLabel(" falcon-01 "), "falcon-01");
});

test("the cards a same-drone rewrite replaces are the drone's others still able to unlock", () => {
  const cards = [
    card({ id: "new", label: "falcon" }),
    card({ id: "old", label: "falcon" }),
    card({ id: "older", label: "falcon", lostAt: "2026-10-01T00:00:00Z" }),
    card({ id: "other", label: "owel-02" }),
    card({ id: "renamed", label: "falcon-02" }),
  ];
  assert.deepEqual(
    retireCandidates(cards, ["falcon"], "new").map((c) => c.id),
    ["old"],
  );
  assert.deepEqual(
    retireCandidates(cards, ["falcon", "falcon-02"], "new").map((c) => c.id),
    ["old", "renamed"],
  );
  assert.deepEqual(retireCandidates(cards, ["racer"], "new"), []);
  assert.deepEqual(
    retireCandidates([card({ id: "x", label: "unnamed card" })], [""], "new").map(
      (c) => c.id,
    ),
    ["x"],
  );
});

const link = (over: Partial<CloudLink>): CloudLink => ({
  account: "op@unit.example",
  on: true,
  sameDrone: false,
  usb: false,
  ...over,
});

test("a signed-in flash links the card to the account by default", () => {
  assert.equal(linkedAccount(link({})), "op@unit.example");
  assert.equal(cloudLinkSummary(link({})), "Links to op@unit.example on first start");
});

test("turning the link off writes no token", () => {
  assert.equal(linkedAccount(link({ on: false })), null);
  assert.equal(cloudLinkSummary(link({ on: false })), "Off");
});

test("without a sign-in there is nothing to link", () => {
  assert.equal(linkedAccount(link({ account: null })), null);
  assert.match(cloudLinkSummary(link({ account: null })), /sign in/);
});

test("a same-drone restore keeps the identity from the backup", () => {
  assert.equal(linkedAccount(link({ sameDrone: true })), null);
  assert.equal(cloudLinkSummary(link({ sameDrone: true })), "Keeps the link from the backup");
});

test("a board written over USB gets no card settings, so no link", () => {
  assert.equal(linkedAccount(link({ usb: true })), null);
  assert.match(cloudLinkSummary(link({ usb: true })), /device page/);
});
