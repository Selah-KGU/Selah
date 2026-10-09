import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { loadTypeScript } from "./load-typescript.mjs";

const { BackendTaskStatusReader } = await loadTypeScript("src/lib/backendTaskStatus.ts");

test("native timestamp batch preserves nulls and converts valid seconds for task indicators", async () => {
  const wire = JSON.parse(await readFile(new URL("./fixtures/cache-batch-reply-wire.json", import.meta.url), "utf8"));
  const tasks = ["notifications", "missing", "large", "schedule_data"].map(key => ({ key, cacheKey: key }));
  const updates = new Map();
  let reads = 0;
  const reader = new BackendTaskStatusReader(tasks, async (keys, includeSchedule) => {
    reads++;
    assert.deepEqual(keys, ["notifications", "missing", "large"]);
    assert.equal(includeSchedule, true);
    return wire.timestamp_batch;
  }, (key, stamp) => updates.set(key, stamp));
  await reader.refresh();
  assert.equal(reads, 1);
  assert.deepEqual(updates, new Map([
    ["notifications", 42000], ["missing", null], ["large", null], ["schedule_data", null],
  ]));
});
