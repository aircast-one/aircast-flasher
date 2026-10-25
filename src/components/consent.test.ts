import { mock, test } from "node:test";
import assert from "node:assert/strict";

import { answerConsent, asksConsent } from "./consent.ts";

const sender = () => mock.fn((_event: string) => undefined);
const events = (send: ReturnType<typeof sender>) =>
  send.mock.calls.map((call) => call.arguments[0]);

test("asks on first run once settings have loaded", () => {
  assert.equal(asksConsent(true, null), true);
});

test("never asks a returning user, even while settings are still loading", () => {
  assert.equal(asksConsent(false, null), false);
  assert.equal(asksConsent(true, true), false);
  assert.equal(asksConsent(true, false), false);
});

test("sharing sends telemetry_enabled only after the answer is saved", async () => {
  const send = sender();
  const save = mock.fn((_patch: { telemetry: boolean }) =>
    Promise.resolve().then(() => {
      assert.equal(send.mock.callCount(), 0, "sent before the install id existed");
      return true;
    }),
  );

  await answerConsent(true, save, send);

  assert.deepEqual(
    save.mock.calls.map((call) => call.arguments[0]),
    [{ telemetry: true }],
  );
  assert.deepEqual(events(send), ["telemetry_enabled"]);
});

test("declining saves the answer and sends nothing", async () => {
  const send = sender();
  const save = mock.fn((_patch: { telemetry: boolean }) => Promise.resolve(true));

  await answerConsent(false, save, send);

  assert.deepEqual(
    save.mock.calls.map((call) => call.arguments[0]),
    [{ telemetry: false }],
  );
  assert.deepEqual(events(send), []);
});

test("a share that failed to save sends nothing", async () => {
  const send = sender();

  await answerConsent(true, () => Promise.resolve(false), send);

  assert.deepEqual(events(send), []);
});
