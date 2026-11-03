import assert from "node:assert/strict";
import { test } from "node:test";

import { timeAgo } from "./format.ts";

test("a backup's age reads in the largest whole unit", () => {
  const now = Date.parse("2026-11-04T12:00:00Z");
  assert.equal(timeAgo("2026-11-04T11:59:30Z", now), "just now");
  assert.equal(timeAgo("2026-11-04T11:15:00Z", now), "45 minutes ago");
  assert.equal(timeAgo("2026-11-04T09:00:00Z", now), "3 hours ago");
  assert.equal(timeAgo("2026-11-01T12:00:00Z", now), "3 days ago");
  assert.equal(timeAgo("not a date", now), "not a date");
});
