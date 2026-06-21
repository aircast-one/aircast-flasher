// Sync a release version (from the git tag) into package.json, tauri.conf.json,
// and the workspace Cargo.toml so the built bundles carry the right version.
//
//   node scripts/sync-version.mjs 1.2.3
import { readFileSync, writeFileSync } from "node:fs";

const version = process.argv[2];
if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
  console.error("usage: node scripts/sync-version.mjs <version>  (e.g. 1.2.3)");
  process.exit(1);
}

// package.json
{
  const path = "package.json";
  const json = JSON.parse(readFileSync(path, "utf8"));
  json.version = version;
  writeFileSync(path, JSON.stringify(json, null, 2) + "\n");
}

// src-tauri/tauri.conf.json — this is the version Tauri stamps into the bundle.
{
  const path = "src-tauri/tauri.conf.json";
  const json = JSON.parse(readFileSync(path, "utf8"));
  json.version = version;
  writeFileSync(path, JSON.stringify(json, null, 2) + "\n");
}

// Cargo.toml — [workspace.package] version (inherited by all crates).
{
  const path = "Cargo.toml";
  const toml = readFileSync(path, "utf8");
  const next = toml.replace(
    /(\[workspace\.package\][\s\S]*?version\s*=\s*")[^"]*(")/,
    `$1${version}$2`,
  );
  if (next === toml) {
    console.error("warning: could not find [workspace.package] version to update");
  }
  writeFileSync(path, next);
}

console.log(`synced version -> ${version}`);
