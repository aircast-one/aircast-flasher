import os from "node:os";
import path from "node:path";

const IDENTIFIER = "one.aircast.flasher";

/// Mirrors Tauri's app_config_dir(): dirs::config_dir()/<identifier>.
export function settingsFile(): string {
  const home = os.homedir();
  const configDir =
    process.platform === "darwin"
      ? path.join(home, "Library", "Application Support")
      : process.platform === "win32"
        ? (process.env.APPDATA ?? path.join(home, "AppData", "Roaming"))
        : (process.env.XDG_CONFIG_HOME ?? path.join(home, ".config"));
  return path.join(configDir, IDENTIFIER, "settings.json");
}
