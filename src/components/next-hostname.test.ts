import assert from "node:assert/strict";
import { test } from "node:test";

import { nextHostname } from "./next-hostname.ts";

test("a generated default is replaced by a fresh generated default", () => {
  const next = nextHostname("aircast-4600ef");
  assert.match(next, /^aircast-[0-9a-f]{6}$/);
  assert.notEqual(next, "aircast-4600ef");
});

test("a numbered name counts up, keeping its padding", () => {
  assert.equal(nextHostname("falcon-01"), "falcon-02");
  assert.equal(nextHostname("falcon-09"), "falcon-10");
  assert.equal(nextHostname("falcon-99"), "falcon-100");
  assert.equal(nextHostname("falcon9"), "falcon10");
});

test("an unnumbered name gets a number rather than reusing the same one", () => {
  assert.equal(nextHostname("falcon"), "falcon-02");
  assert.equal(nextHostname("  falcon  "), "falcon-02");
});

test("an empty name falls back to a generated default", () => {
  assert.match(nextHostname(""), /^aircast-[0-9a-f]{6}$/);
});

test("flashing a batch never repeats a hostname", () => {
  const names = Array.from({ length: 20 }).reduce(
    (acc: string[]) => [...acc, nextHostname(acc[acc.length - 1])],
    ["falcon-01"],
  );
  assert.equal(new Set(names).size, names.length);
});
