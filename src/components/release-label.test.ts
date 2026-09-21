import assert from "node:assert/strict";
import { test } from "node:test";

import { releaseLabel } from "./release-label.ts";

test("a stable release carries no label", () => {
  assert.equal(releaseLabel({ version: "v0.3.4", prerelease: false }), "");
});

test("a beta on the stable channel says beta, not pre-release", () => {
  assert.equal(
    releaseLabel({ version: "v0.3.4-beta.1", prerelease: true }),
    " · beta",
  );
});

test("other pre-releases keep the generic label", () => {
  assert.equal(
    releaseLabel({ version: "v0.3.4-dev.3", prerelease: true }),
    " · pre-release",
  );
});
