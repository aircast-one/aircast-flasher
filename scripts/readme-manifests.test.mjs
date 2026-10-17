import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

const manifestPaths = (text) => new Set(text.match(/\/[a-z]+\/(?:releases|latest)\.json/g) ?? []);

const fetchedByCode = new Set([
  ...manifestPaths(read("src-tauri/src/flasher/releases.rs")),
  ...manifestPaths(read("scripts/configure-updater.mjs")),
  ...manifestPaths(read(".github/workflows/desktop-release.yml")),
]);

test("every manifest the README names is one the code or release workflow uses", () => {
  const documented = [...manifestPaths(read("README.md"))];
  assert.ok(documented.length > 0, "README names no manifest");
  assert.deepEqual(
    documented.filter((path) => !fetchedByCode.has(path)),
    [],
  );
});

test("the README names the image manifest list_releases fetches", () => {
  assert.ok(fetchedByCode.has("/lite/releases.json"));
  assert.ok(manifestPaths(read("README.md")).has("/lite/releases.json"));
});
