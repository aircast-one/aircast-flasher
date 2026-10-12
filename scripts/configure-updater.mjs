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
