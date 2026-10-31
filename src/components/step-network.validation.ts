export interface FieldProblem {
  field: string;
  message: string;
}

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
  authKey: string,
): boolean {
  return selfHosted && controlServer.trim() === "" && authKey.trim() !== "";
}

export function joinsTailnet(authKey: string, controlServer: string): boolean {
  return authKey.trim() !== "" && isValidControlServer(controlServer);
}

export function fightModeUnreachable(
  fightMode: boolean,
  authKey: string,
  controlServer: string,
): boolean {
  return fightMode && !joinsTailnet(authKey, controlServer);
}

export function needsSsid(noWifi: boolean, ssid: string): boolean {
  return !noWifi && ssid.trim() === "";
}

const SSH_KEY_PATTERN = /^[a-z][a-z0-9@.-]*\s+[A-Za-z0-9+/]+=*(\s.*)?$/i;

export function isInvalidSshKey(value: string): boolean {
  const v = value.trim();
  return v !== "" && !SSH_KEY_PATTERN.test(v);
}

const RAW_PSK = /^[0-9a-f]{64}$/i;

export function isInvalidWifiPassword(value: string): boolean {
  return (
    value !== "" &&
    (value.length < 8 || value.length > 63) &&
    !RAW_PSK.test(value)
  );
}

export const MIN_DEVICE_PASSWORD = 8;
export const MIN_FIGHT_PASSWORD = 12;

export const minDevicePassword = (fighting: boolean): number =>
  fighting ? MIN_FIGHT_PASSWORD : MIN_DEVICE_PASSWORD;

export function isWeakDevicePassword(value: string, fighting: boolean): boolean {
  return fighting
    ? value.trim().length < MIN_FIGHT_PASSWORD
    : value !== "" && value.length < MIN_DEVICE_PASSWORD;
}

export const networkProblems = (p: {
  noWifi: boolean;
  ssid: string;
  password: string;
  hostname: string;
}): FieldProblem[] => [
  ...(needsSsid(p.noWifi, p.ssid)
    ? [
        {
          field: "ssid",
          message:
            "Enter the WiFi network the device joins, or choose Ethernet or cellular only",
        },
      ]
    : []),
  ...(!p.noWifi && isInvalidWifiPassword(p.password)
    ? [{ field: "password", message: "Make the WiFi password 8–63 characters" }]
    : []),
  ...(p.hostname.trim() === ""
    ? [{ field: "hostname", message: "Enter a hostname" }]
    : isValidHostname(p.hostname)
      ? []
      : [
          {
            field: "hostname",
            message:
              "Use only lowercase letters, numbers and hyphens in the hostname",
          },
        ]),
];
