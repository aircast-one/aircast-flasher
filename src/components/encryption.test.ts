import { test } from "node:test";
import assert from "node:assert/strict";

import {
  encryptionIncomplete,
  isCaptured,
  releaseSupportsEncryption,
} from "./encryption.ts";

const cap = { url: "http://a", adv: "{}" };
const uncap = { url: "http://b", adv: "" };

test("disabled encryption is never incomplete", () => {
  assert.equal(
    encryptionIncomplete({ enabled: false, servers: [], threshold: 1, cloud: false, recoverySaved: false }),
    false,
  );
});

test("needs at least one server with a URL", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [], threshold: 1, cloud: false, recoverySaved: true }),
    true,
  );
});

test("every URL row must be captured", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [cap, uncap], threshold: 1, cloud: false, recoverySaved: true }),
    true,
  );
});

test("threshold above the server count is incomplete", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [cap], threshold: 2, cloud: false, recoverySaved: true }),
    true,
  );
});

test("threshold below one is incomplete", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [cap], threshold: 0, cloud: false, recoverySaved: true }),
    true,
  );
});

test("an unsaved recovery key is incomplete", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [cap], threshold: 1, cloud: false, recoverySaved: false }),
    true,
  );
});

test("captured servers, valid threshold, saved key is complete", () => {
  assert.equal(
    encryptionIncomplete({
      enabled: true,
      servers: [cap, { url: "http://c", adv: "{}" }],
      threshold: 2,
      cloud: false,
      recoverySaved: true,
    }),
    false,
  );
});

test("isCaptured needs both URL and advertisement", () => {
  assert.equal(isCaptured(cap), true);
  assert.equal(isCaptured(uncap), false);
  assert.equal(isCaptured({ url: "", adv: "{}" }), false);
});

test("the Aircast Cloud key server alone is enough", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [], threshold: 1, cloud: true, recoverySaved: true }),
    false,
  );
});

test("the cloud counts toward the threshold", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [cap], threshold: 2, cloud: true, recoverySaved: true }),
    false,
  );
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [cap], threshold: 3, cloud: true, recoverySaved: true }),
    true,
  );
});

test("an uncaptured unit server still blocks, cloud or not", () => {
  assert.equal(
    encryptionIncomplete({ enabled: true, servers: [uncap], threshold: 1, cloud: true, recoverySaved: true }),
    true,
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
