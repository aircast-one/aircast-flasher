import fs from "node:fs";
import path from "node:path";

import { settingsFile } from "./e2e/settings-file";

const BINARY =
  process.env.FLASHER_BINARY ??
  path.resolve(
    import.meta.dirname,
    "target",
    process.env.FLASHER_PROFILE ?? "debug",
    process.platform === "win32" ? "aircast-flasher.exe" : "aircast-flasher",
  );

const BACKUP = `${settingsFile()}.e2e-backup`;

interface TauriCapability {
  browserName: "tauri";
  "tauri:options": { application: string };
}

const capabilities: TauriCapability[] = [
  { browserName: "tauri", "tauri:options": { application: BINARY } },
];

export const config: WebdriverIO.Config = {
  runner: "local",
  specs: ["./e2e/**/*.e2e.ts"],
  maxInstances: 1,
  capabilities: capabilities as WebdriverIO.Config["capabilities"],
  services: [
    [
      "@wdio/tauri-service",
      { captureBackendLogs: true, captureFrontendLogs: true },
    ],
  ],
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  logLevel: "error",
  specFileRetries: 1,
  waitforTimeout: 15_000,

  // The app reads its settings once at startup, so the suite gets a pristine
  // wizard by moving the operator's real settings aside before the app launches
  // — no in-app reload, which WebKitGTK does not survive on a tauri:// origin.
  onPrepare() {
    const file = settingsFile();
    if (fs.existsSync(file)) fs.renameSync(file, BACKUP);
  },

  onComplete() {
    const file = settingsFile();
    if (fs.existsSync(BACKUP)) {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      fs.renameSync(BACKUP, file);
    } else {
      fs.rmSync(file, { force: true });
    }
  },
};
