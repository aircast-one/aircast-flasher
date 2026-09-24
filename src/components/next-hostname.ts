import { defaultHostname } from "@/components/default-hostname";

const TRAILING_NUMBER = /^(.*?)(\d+)$/;
const MAX_ADVANCE = 50;

export function nextHostname(current: string): string {
  const name = current.trim();
  if (name === "") return defaultHostname();

  const match = TRAILING_NUMBER.exec(name);
  if (!match) return `${name}-02`;

  const [, stem, digits] = match;
  const incremented = String(Number(digits) + 1);
  return `${stem}${incremented.padStart(digits.length, "0")}`;
}

/// The next name after `current` that nothing already holds. Counting up from a
/// custom stem keeps the operator's naming (`falcon-01` -> `falcon-02`), which
/// a bare default would throw away.
export function nextFreeHostname(
  current: string,
  taken: readonly string[],
): string {
  const used = new Set(taken.map((name) => name.trim().toLowerCase()));
  return Array.from({ length: MAX_ADVANCE }).reduce<string>(
    (name) => (used.has(name.toLowerCase()) ? nextHostname(name) : name),
    nextHostname(current),
  );
}
