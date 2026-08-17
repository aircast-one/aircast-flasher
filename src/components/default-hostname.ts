export function defaultHostname(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(3));
  const suffix = Array.from(bytes)
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  return `aircast-${suffix}`;
}
