import fs from "node:fs";
import path from "node:path";

import { e2eConfigDir } from "./e2e/settings-file";

const BINARY =
  process.env.FLASHER_BINARY ??
  path.resolve(
    import.meta.dirname,
    "target",
    process.env.FLASHER_PROFILE ?? "debug",
    process.platform === "win32" ? "aircast-flasher.exe" : "aircast-flasher",
  );

const CONFIG_DIR = e2eConfigDir();

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
      {
        driverProvider: process.platform === "linux" ? "external" : "embedded",
        autoInstallTauriDriver: process.platform === "linux",
        captureBackendLogs: true,
        captureFrontendLogs: true,
      },
    ],
  ],
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  logLevel: "error",
  specFileRetries: 1,
  waitforTimeout: 15_000,

  onComplete() {
    fs.rmSync(CONFIG_DIR, { recursive: true, force: true });
  },
};
