import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

const manifestPaths = (text) =>
  new Set(text.match(/\/[a-z0-9-]+\/(?:releases|latest)\.json/g) ?? []);

const boardCatalogs = [
  ...read("crates/flasher-ops/src/releases.rs").matchAll(
    /catalog: "([a-z0-9-]+)"|rockchip\(\s*(?:"[^"]*",\s*){4}(?:true|false),\s*"([a-z0-9-]+)"/g,
  ),
].map(([, pi, board]) => `/${pi ?? board}/releases.json`);

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

test("the README names the Pi's image manifest and a real board example", () => {
  const documented = manifestPaths(read("README.md"));
  assert.ok(boardCatalogs.includes("/lite/releases.json"));
  assert.ok(documented.has("/lite/releases.json"));
  assert.ok(
    [...documented].some((path) => path !== "/lite/releases.json" && boardCatalogs.includes(path)),
  );
});
