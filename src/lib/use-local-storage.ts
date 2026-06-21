import { useEffect, useState, type Dispatch, type SetStateAction } from "react";

/**
 * A `useState` for a string that persists to the WebView's localStorage, so
 * values (WiFi SSID/password, hostname) survive app restarts and don't have to
 * be re-entered each flash. Stored locally on this machine only. The setter
 * supports the same updater-function form as `useState`.
 */
export function useLocalStorage(
  key: string,
  initial: string,
): [string, Dispatch<SetStateAction<string>>] {
  const [value, setValue] = useState<string>(() => {
    try {
      return localStorage.getItem(key) ?? initial;
    } catch {
      return initial;
    }
  });

  useEffect(() => {
    try {
      localStorage.setItem(key, value);
    } catch {
      // ignore storage failures (e.g. private mode)
    }
  }, [key, value]);

  return [value, setValue];
}
