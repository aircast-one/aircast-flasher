// Configure the Tauri updater for a release build.
//
// Two release-only concerns live here, both keyed off DOWNLOADS_URL (set by the
// release workflow, unset for local/CI builds):
//
//   1. Endpoint  — each release channel (stable / staging / development) serves
//      its own latest.json under its own downloads domain, so the built bundle
//      must embed the endpoint for the channel it's being built for.
//   2. Signed artifacts — bundle.createUpdaterArtifacts is committed as `false`
//      so a plain `tauri build` (CI checks, local dev) doesn't demand a signing
//      key. It's flipped on here so release bundles emit the signed .sig updater
//      artifacts the embedded pubkey verifies against.
//
//   DOWNLOADS_URL=https://downloads.dev.aircast.one node scripts/configure-updater.mjs
//
// No-ops gracefully (exit 0) when DOWNLOADS_URL is unset — e.g. local/CI builds.
import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const CONFIG_PATH = path.join(__dirname, "..", "src-tauri", "tauri.conf.json");

const downloadsUrl = process.env.DOWNLOADS_URL;
if (!downloadsUrl) {
  console.log("DOWNLOADS_URL not set — keeping default updater endpoint.");
  process.exit(0);
}

const endpoint = `${downloadsUrl.replace(/\/+$/, "")}/flasher/latest.json`;

const config = JSON.parse(readFileSync(CONFIG_PATH, "utf8"));
if (!config.plugins?.updater) {
  console.error("No plugins.updater block in tauri.conf.json");
  process.exit(1);
}
config.plugins.updater.endpoints = [endpoint];
config.bundle ??= {};
config.bundle.createUpdaterArtifacts = true;
writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2) + "\n");

console.log(`updater endpoint -> ${endpoint}`);
console.log("createUpdaterArtifacts -> true");
