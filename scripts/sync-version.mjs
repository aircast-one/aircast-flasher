// Sync a release version (from the git tag) into package.json, tauri.conf.json,
// and the workspace Cargo.toml so the built bundles carry the right version.
// With --check, verify instead of write and exit 1 on any mismatch.
//
//   node scripts/sync-version.mjs 1.2.3
//   node scripts/sync-version.mjs 1.2.3 --check
import { readFileSync, writeFileSync } from "node:fs";

const args = process.argv.slice(2);
const check = args.includes("--check");
const version = args.find((a) => a !== "--check");
if (!version || !/^\d+\.\d+\.\d+/.test(version)) {
  console.error(
    "usage: node scripts/sync-version.mjs <version> [--check]  (e.g. 1.2.3)",
  );
  process.exit(1);
}

const mismatches = [];

const jsonVersion = (path, current) => {
  if (check) {
    if (current !== version) mismatches.push(`${path}: ${current}`);
    return false;
  }
  return current !== version;
};

// package.json
{
  const path = "package.json";
  const json = JSON.parse(readFileSync(path, "utf8"));
  if (jsonVersion(path, json.version)) {
    json.version = version;
    writeFileSync(path, JSON.stringify(json, null, 2) + "\n");
  }
}

// src-tauri/tauri.conf.json — this is the version Tauri stamps into the bundle.
{
  const path = "src-tauri/tauri.conf.json";
  const json = JSON.parse(readFileSync(path, "utf8"));
  if (jsonVersion(path, json.version)) {
    json.version = version;
    writeFileSync(path, JSON.stringify(json, null, 2) + "\n");
  }
}

// Cargo.toml — [workspace.package] version (inherited by all crates).
{
  const path = "Cargo.toml";
  const toml = readFileSync(path, "utf8");
  const re = /(\[workspace\.package\][\s\S]*?version\s*=\s*")([^"]*)(")/;
  const current = toml.match(re)?.[2];
  if (current === undefined) {
    console.error("could not find [workspace.package] version");
    process.exit(1);
  }
  if (check) {
    if (current !== version) mismatches.push(`${path}: ${current}`);
  } else if (current !== version) {
    writeFileSync(path, toml.replace(re, `$1${version}$3`));
  }
}

if (check) {
  if (mismatches.length > 0) {
    console.error(`repo version does not match tag ${version}:`);
    mismatches.forEach((m) => console.error(`  ${m}`));
    console.error("release with scripts/release.sh, which commits the bump");
    process.exit(1);
  }
  console.log(`version ${version} verified`);
} else {
  console.log(`synced version -> ${version}`);
}
