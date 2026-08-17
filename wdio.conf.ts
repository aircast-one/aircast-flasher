import path from "node:path";

const BINARY =
  process.env.FLASHER_BINARY ??
  path.resolve(
    import.meta.dirname,
    "target",
    process.env.FLASHER_PROFILE ?? "debug",
    process.platform === "win32" ? "aircast-flasher.exe" : "aircast-flasher",
  );

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
  services: ["@wdio/tauri-service"],
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: { ui: "bdd", timeout: 120_000 },
  logLevel: "error",
  specFileRetries: 1,
  waitforTimeout: 15_000,
};
