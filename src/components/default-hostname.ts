import { randomHex } from "@/lib/random-id";

export const GENERATED_HOSTNAME = /^aircast-[0-9a-f]{6}$/;

export function defaultHostname(): string {
  return `aircast-${randomHex(3)}`;
}
