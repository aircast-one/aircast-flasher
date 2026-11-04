import type { SshMode } from "@/types";
import type { FlashProgressState, SourceKind } from "./wizard-types.ts";

export type DebugRequest = Readonly<Record<string, unknown>>;
export type DebugReply = Readonly<Record<string, unknown>>;

export const DEBUG_PAGES = ["os", "network", "access", "storage", "devices", "cloud"] as const;
export type DebugPage = (typeof DEBUG_PAGES)[number];

const TEXT_FIELDS = ["hostname", "ssid", "wifiPassword", "authKey", "devicePassword"] as const;
const TOGGLE_FIELDS = ["noWifi", "remoteAccess", "encryption", "fightMode"] as const;
const SSH_MODES: readonly SshMode[] = ["key-only", "password", "disabled"];
const ACTIONS = ["state", "consent", "goto", "set", "flash", "cancel"] as const;

export type TextField = (typeof TEXT_FIELDS)[number];
export type ToggleField = (typeof TOGGLE_FIELDS)[number];

export interface DebugDisk {
  readonly path: string;
  readonly name: string;
  readonly size: number;
}

export interface DebugView {
  readonly page: DebugPage | "consent" | "write";
  readonly boards: readonly string[];
  readonly board: string | null;
  readonly sourceKind: SourceKind;
  readonly versions: readonly string[];
  readonly version: string | null;
  readonly hostname: string;
  readonly ssid: string;
  readonly noWifi: boolean;
  readonly wifiPasswordSet: boolean;
  readonly remoteAccess: boolean;
  readonly authKeySet: boolean;
  readonly sshMode: SshMode | null;
  readonly devicePasswordSet: boolean;
  readonly encryption: boolean;
  readonly fightMode: boolean;
  readonly disks: readonly DebugDisk[];
  readonly disk: string;
  readonly problems: readonly string[];
  readonly canFlash: boolean;
  readonly status: string;
  readonly progress: FlashProgressState;
  readonly writing: boolean;
  readonly written: boolean;
  readonly error: string | null;
}

export interface DebugControls {
  readonly view: DebugView;
  readonly consent: (share: boolean) => void;
  readonly goto: (page: DebugPage) => void;
  readonly board: (id: string) => void;
  readonly version: (version: string) => void;
  readonly disk: (path: string) => void;
  readonly text: (field: TextField, value: string) => void;
  readonly toggle: (field: ToggleField, on: boolean) => void;
  readonly sshMode: (mode: SshMode) => void;
  readonly flash: () => void;
  readonly cancel: () => void;
}

const refused = (reason: string, extra: DebugReply = {}): DebugReply => ({ ok: false, reason, ...extra });

const isOneOf = <T extends string>(options: readonly T[], value: unknown): value is T =>
  typeof value === "string" && (options as readonly string[]).includes(value);

const settingProblem = (key: string, value: unknown, view: DebugView): string | null =>
  key === "board"
    ? isOneOf(view.boards, value) ? null : `board must be one of: ${view.boards.join(", ")}`
    : key === "version"
      ? isOneOf(view.versions, value) ? null : `version must be one of: ${view.versions.join(", ")}`
      : key === "disk"
        ? isOneOf(view.disks.map((d) => d.path), value)
          ? null
          : `disk must be one of the offered cards: ${view.disks.map((d) => d.path).join(", ") || "none"}`
        : key === "sshMode"
          ? isOneOf(SSH_MODES, value) ? null : `sshMode must be one of: ${SSH_MODES.join(", ")}`
          : isOneOf(TEXT_FIELDS, key)
            ? typeof value === "string" ? null : `${key} must be text`
            : isOneOf(TOGGLE_FIELDS, key)
              ? typeof value === "boolean" ? null : `${key} must be true or false`
              : `unknown setting ${key}`;

const setters = (controls: DebugControls): Readonly<Record<string, (value: unknown) => void>> => ({
  board: (value) => controls.board(value as string),
  version: (value) => controls.version(value as string),
  disk: (value) => controls.disk(value as string),
  sshMode: (value) => controls.sshMode(value as SshMode),
  ...Object.fromEntries(TEXT_FIELDS.map((field) => [field, (value: unknown) => controls.text(field, value as string)])),
  ...Object.fromEntries(TOGGLE_FIELDS.map((field) => [field, (value: unknown) => controls.toggle(field, value as boolean)])),
});

const applySetting = (key: string, value: unknown, controls: DebugControls): string => {
  setters(controls)[key](value);
  return key;
};

const set = (request: DebugRequest, controls: DebugControls): DebugReply => {
  const settings = Object.entries(request).filter(([key]) => key !== "action");
  const problems = settings
    .map(([key, value]) => settingProblem(key, value, controls.view))
    .filter((problem): problem is string => problem !== null);
  return settings.length === 0
    ? refused("set needs at least one setting")
    : problems.length > 0
      ? refused(problems.join("; "))
      : { ok: true, applied: settings.map(([key, value]) => applySetting(key, value, controls)) };
};

const consent = (request: DebugRequest, controls: DebugControls): DebugReply => {
  if (typeof request.share !== "boolean") return refused("consent needs share: true or false");
  controls.consent(request.share);
  return { ok: true };
};

const goto = (request: DebugRequest, controls: DebugControls): DebugReply => {
  if (!isOneOf(DEBUG_PAGES, request.page)) return refused(`page must be one of: ${DEBUG_PAGES.join(", ")}`);
  if (controls.view.writing) return refused("A card is being written; cancel it first");
  controls.goto(request.page);
  return { ok: true };
};

const flash = (controls: DebugControls): DebugReply => {
  if (!controls.view.canFlash) return refused("Not ready to write", { problems: controls.view.problems });
  controls.flash();
  return { ok: true };
};

const cancel = (controls: DebugControls): DebugReply => {
  if (!controls.view.writing) return refused("Nothing is being written");
  controls.cancel();
  return { ok: true };
};

export const runDebugRequest = (request: DebugRequest, controls: DebugControls): DebugReply =>
  request.action === "state"
    ? { ok: true, ...controls.view }
    : request.action === "consent"
      ? consent(request, controls)
      : request.action === "goto"
        ? goto(request, controls)
        : request.action === "set"
          ? set(request, controls)
          : request.action === "flash"
            ? flash(controls)
            : request.action === "cancel"
              ? cancel(controls)
              : refused(`action must be one of: ${ACTIONS.join(", ")}`);
