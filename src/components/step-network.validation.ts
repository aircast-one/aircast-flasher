const HOSTNAME_PATTERN = /^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$/;

export function isValidHostname(value: string): boolean {
  return HOSTNAME_PATTERN.test(value.trim());
}

export function isValidControlServer(value: string): boolean {
  const v = value.trim();
  return v === "" || /^https?:\/\/.+/.test(v);
}

export function needsAuthKey(controlServer: string, authKey: string): boolean {
  return controlServer.trim() !== "" && authKey.trim() === "";
}

// A key-type token (covers ssh-*, ecdsa-*, sk-*, and *-cert-v01@openssh.com
// certificate keys), then the base64 body, then an optional comment. Permissive
// on the type so valid variants aren't rejected, but still requires the
// type<space>base64 shape so plain text / URLs don't pass.
const SSH_KEY_PATTERN = /^[a-z][a-z0-9@.-]*\s+[A-Za-z0-9+/]+=*(\s.*)?$/i;

// True when a non-empty value does not look like an SSH public key. Empty is
// not "invalid" — it's a no-op that keeps the image default.
export function isInvalidSshKey(value: string): boolean {
  const v = value.trim();
  return v !== "" && !SSH_KEY_PATTERN.test(v);
}

export const MIN_DEVICE_PASSWORD = 8;

// True when a non-empty device password is too short. Empty is a no-op that
// keeps the image default.
export function isWeakDevicePassword(value: string): boolean {
  return value !== "" && value.length < MIN_DEVICE_PASSWORD;
}
