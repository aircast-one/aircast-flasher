// Point the Tauri updater at the right channel's latest.json.
//
// Each release channel (stable / staging / development) serves its own
// latest.json under its own downloads domain, so the built bundle must embed
// the endpoint for the channel it's being built for. Reads DOWNLOADS_URL from
// the environment (set by the release workflow) and rewrites
// plugins.updater.endpoints in src-tauri/tauri.conf.json.
//
//   DOWNLOADS_URL=https://downloads.dev.aircast.one node scripts/configure-updater.mjs
//
// No-ops gracefully (exit 0) when DOWNLOADS_URL is unset — e.g. local builds.
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
writeFileSync(CONFIG_PATH, JSON.stringify(config, null, 2) + "\n");

console.log(`updater endpoint -> ${endpoint}`);
