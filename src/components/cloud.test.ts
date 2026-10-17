import { test } from "node:test";
import assert from "node:assert/strict";

import { cardLabel, cardState, waitForSignIn, withCloudServer } from "./cloud.ts";
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
