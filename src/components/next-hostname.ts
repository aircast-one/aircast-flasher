import { defaultHostname, GENERATED_HOSTNAME } from "@/components/default-hostname";

const TRAILING_NUMBER = /^(.*?)(\d+)$/;

export function nextHostname(current: string): string {
  const name = current.trim();
  if (name === "" || GENERATED_HOSTNAME.test(name)) return defaultHostname();

  const match = TRAILING_NUMBER.exec(name);
  if (!match) return `${name}-02`;

  const [, stem, digits] = match;
  const incremented = String(Number(digits) + 1);
  return `${stem}${incremented.padStart(digits.length, "0")}`;
}
