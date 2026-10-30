import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

const manifestPaths = (text) =>
  new Set(text.match(/\/[a-z0-9-]+\/(?:releases|latest)\.json/g) ?? []);

const boardCatalogs = [
  ...read("crates/flasher-ops/src/releases.rs").matchAll(/catalog: "([a-z0-9-]+)"/g),
].map(([, catalog]) => `/${catalog}/releases.json`);

const fetchedByCode = new Set([
  ...boardCatalogs,
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

test("the README names the image manifest of every board list_releases fetches", () => {
  assert.ok(boardCatalogs.includes("/lite/releases.json"));
  const documented = manifestPaths(read("README.md"));
  assert.deepEqual(
    boardCatalogs.filter((path) => !documented.has(path)),
    [],
  );
});
