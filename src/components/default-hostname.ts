export const GENERATED_HOSTNAME = /^aircast-\d{2,}$/;

const MAX_GENERATED = 999;

const numbered = (n: number) => `aircast-${String(n).padStart(2, "0")}`;

/// The lowest `aircast-NN` not already taken. A number says which device this
/// is; the random hex it replaced said nothing, and existed only because the
/// app could not see the fleet. It can now — `taken` is the tailnet.
export function defaultHostname(taken: readonly string[] = []): string {
  const used = new Set(taken.map((name) => name.trim().toLowerCase()));
  return (
    Array.from({ length: MAX_GENERATED }, (_, i) => numbered(i + 1)).find(
      (candidate) => !used.has(candidate),
    ) ?? numbered(1)
  );
}
