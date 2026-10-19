import { test } from "node:test";
import assert from "node:assert/strict";

import {
  encryptionProblems,
  isCaptured,
  releaseSupportsFight,
  releaseSupportsEncryption,
} from "./encryption.ts";

const cap = { url: "http://a", adv: "{}" };
const uncap = { url: "http://b", adv: "" };

const fields = (p: Parameters<typeof encryptionProblems>[0]) =>
  encryptionProblems(p).map((problem) => problem.field);

test("disabled encryption is never incomplete", () => {
  assert.deepEqual(
    fields({ enabled: false, servers: [], threshold: 1, cloud: false, recoverySaved: false }),
    [],
  );
});

test("needs at least one server with a URL", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [], threshold: 1, cloud: false, recoverySaved: true }),
    ["tang-server-0"],
  );
});

test("every URL row must be captured", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [cap, uncap], threshold: 1, cloud: false, recoverySaved: true }),
    ["tang-server-1"],
  );
});

test("threshold above the server count is incomplete", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [cap], threshold: 2, cloud: false, recoverySaved: true }),
    ["threshold"],
  );
});

test("threshold below one is incomplete", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [cap], threshold: 0, cloud: false, recoverySaved: true }),
    ["threshold"],
  );
});

test("an unsaved recovery key is incomplete", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [cap], threshold: 1, cloud: false, recoverySaved: false }),
    ["recovery-key"],
  );
});

test("captured servers, valid threshold, saved key is complete", () => {
  assert.deepEqual(
    fields({
      enabled: true,
      servers: [cap, { url: "http://c", adv: "{}" }],
      threshold: 2,
      cloud: false,
      recoverySaved: true,
    }),
    [],
  );
});

test("names every unfinished part, in page order", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [uncap], threshold: 1, cloud: false, recoverySaved: false }),
    ["tang-server-0", "recovery-key"],
  );
});

test("isCaptured needs both URL and advertisement", () => {
  assert.equal(isCaptured(cap), true);
  assert.equal(isCaptured(uncap), false);
  assert.equal(isCaptured({ url: "", adv: "{}" }), false);
});

test("the Aircast Cloud key server alone is enough", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [], threshold: 1, cloud: true, recoverySaved: true }),
    [],
  );
});

test("the cloud counts toward the threshold", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [cap], threshold: 2, cloud: true, recoverySaved: true }),
    [],
  );
  assert.deepEqual(
    fields({ enabled: true, servers: [cap], threshold: 3, cloud: true, recoverySaved: true }),
    ["threshold"],
  );
});

test("an uncaptured unit server still blocks, cloud or not", () => {
  assert.deepEqual(
    fields({ enabled: true, servers: [uncap], threshold: 1, cloud: true, recoverySaved: true }),
    ["tang-server-0"],
  );
});

test("Aircast images before v0.3.6 can't encrypt, so the flasher must not offer it", () => {
  const supports = releaseSupportsEncryption;
  assert.equal(supports("v0.3.5"), false);
  assert.equal(supports("v0.3.5-beta.2"), false);
  assert.equal(supports("v0.3.6-dev.1"), true);
  assert.equal(supports("v0.3.6-beta.1"), true);
  assert.equal(supports("v0.3.6"), true);
  assert.equal(supports("v0.4.0"), true);
  assert.equal(supports("not-a-version"), false);
});

test("fight mode needs an image whose aircastd reads the fight-mode flag", () => {
  assert.equal(releaseSupportsFight("v0.3.5"), false);
  assert.equal(releaseSupportsFight("v0.3.6-beta.1"), true);
  assert.equal(releaseSupportsFight("v0.4.0"), true);
  assert.equal(releaseSupportsFight("not-a-version"), false);
});
