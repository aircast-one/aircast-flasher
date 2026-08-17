import { randomHex } from "@/lib/random-id";

export function defaultHostname(): string {
  return `aircast-${randomHex(3)}`;
}
