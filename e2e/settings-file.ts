import fs from "node:fs";
import os from "node:os";
import path from "node:path";

export const E2E_CONFIG_DIR = "FLASHER_E2E_CONFIG_DIR";

export function e2eConfigDir(): string {
  process.env[E2E_CONFIG_DIR] ??= fs.mkdtempSync(
    path.join(os.tmpdir(), "flasher-e2e-"),
  );
  return process.env[E2E_CONFIG_DIR];
}

export const settingsFile = (): string =>
  path.join(e2eConfigDir(), "settings.json");
