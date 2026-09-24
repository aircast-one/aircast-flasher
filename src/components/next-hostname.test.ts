import assert from "node:assert/strict";
import { test } from "node:test";

import { nextFreeHostname, nextHostname } from "./next-hostname.ts";

test("a generated default counts up like any other numbered name", () => {
  assert.equal(nextHostname("aircast-01"), "aircast-02");
  assert.equal(nextHostname("aircast-09"), "aircast-10");
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
  assert.equal(nextHostname(""), "aircast-01");
});

test("flashing a batch never repeats a hostname", () => {
  const names = Array.from({ length: 20 }).reduce(
    (acc: string[]) => [...acc, nextHostname(acc[acc.length - 1])],
    ["falcon-01"],
  );
  assert.equal(new Set(names).size, names.length);
});

test("counting up skips names already taken, keeping the operator's stem", () => {
  assert.equal(nextFreeHostname("falcon-01", []), "falcon-02");
  assert.equal(nextFreeHostname("falcon-01", ["falcon-02"]), "falcon-03");
  assert.equal(
    nextFreeHostname("falcon-01", ["falcon-02", "FALCON-03"]),
    "falcon-04",
  );
  assert.equal(nextFreeHostname("aircast-01", ["aircast-02"]), "aircast-03");
});
