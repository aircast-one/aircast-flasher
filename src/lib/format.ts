export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "—";
  const gb = bytes / 1000 / 1000 / 1000;
  if (gb >= 1) return `${gb.toFixed(gb >= 10 ? 0 : 1)} GB`;
  const mb = bytes / 1000 / 1000;
  return `${mb.toFixed(0)} MB`;
}

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function formatDateTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleString(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      });
}

const RELATIVE = new Intl.RelativeTimeFormat("en", { numeric: "always" });
const AGE_UNITS = [
  ["day", 86_400_000],
  ["hour", 3_600_000],
  ["minute", 60_000],
] as const;

export function timeAgo(iso: string, now: number): string {
  const elapsed = now - Date.parse(iso);
  const unit = AGE_UNITS.find(([, ms]) => elapsed >= ms);
  return Number.isNaN(elapsed)
    ? iso
    : unit === undefined
      ? "just now"
      : RELATIVE.format(-Math.floor(elapsed / unit[1]), unit[0]);
}

export const backupLabel = (backup: { hostname: string; created: string }) =>
  `${backup.hostname} · ${formatDateTime(backup.created)}`;

export function formatSpeed(bps: number): string {
  if (!Number.isFinite(bps) || bps <= 0) return "";
  const mbps = bps / 1000 / 1000;
  return `${mbps.toFixed(1)} MB/s`;
}

export function errorMessage(e: unknown, fallback: string): string {
  if (typeof e === "string") return e;
  if (e instanceof Error) return e.message;
  return fallback;
}
