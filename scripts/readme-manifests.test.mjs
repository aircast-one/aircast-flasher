import test from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";

const read = (path) => readFileSync(new URL(`../${path}`, import.meta.url), "utf8");

const manifestPaths = (text) =>
  new Set(text.match(/\/[a-z0-9-]+\/(?:releases|latest)\.json/g) ?? []);

const imageRepos = new Set(
  [...read("crates/flasher-ops/src/releases.rs").matchAll(/"(aircast-one\/[a-z0-9-]+)"/g)].map(
    ([, repo]) => repo,
  ),
);

const fetchedByCode = new Set([
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

test("the README names every repository the images are released from", () => {
  const readme = read("README.md");
  assert.deepEqual([...imageRepos].sort(), ["aircast-one/aircast-armbian", "aircast-one/aircast-pi-gen"]);
  assert.deepEqual([...imageRepos].filter((repo) => !readme.includes(repo)), []);
});
