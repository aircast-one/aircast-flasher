const HOSTNAME_PATTERN = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;

export function isValidHostname(value: string): boolean {
  return HOSTNAME_PATTERN.test(value.trim());
}

export function isValidControlServer(value: string): boolean {
  const v = value.trim();
  return v === "" || /^https?:\/\/.+/.test(v);
}

export function needsControlServer(
  selfHosted: boolean,
  controlServer: string,
): boolean {
  return selfHosted && controlServer.trim() === "";
}

export function needsAuthKey(controlServer: string, authKey: string): boolean {
  return controlServer.trim() !== "" && authKey.trim() === "";
}

export function needsSsid(noWifi: boolean, ssid: string): boolean {
  return !noWifi && ssid.trim() === "";
}

const SSH_KEY_PATTERN = /^[a-z][a-z0-9@.-]*\s+[A-Za-z0-9+/]+=*(\s.*)?$/i;

export function isInvalidSshKey(value: string): boolean {
  const v = value.trim();
  return v !== "" && !SSH_KEY_PATTERN.test(v);
}

export const MIN_DEVICE_PASSWORD = 8;

export function isWeakDevicePassword(value: string): boolean {
  return value !== "" && value.length < MIN_DEVICE_PASSWORD;
}
