// Synthetic notification bursts through the immediate predecessor and current queue.
import assert from "node:assert/strict";
import { loadTypeScript } from "../tests/load-typescript.mjs";

const implementations = await Promise.all([
  loadTypeScript("tests/fixtures/cache-sync-queue-before.ts"),
  loadTypeScript("src/lib/cacheSyncQueue.ts"),
]);
const deferred = () => {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
};
const median = values => [...values].sort((a,b) => a-b)[Math.floor(values.length/2)];

async function run(createCacheSyncQueue, requestsPerBurst) {
  const gates = [deferred(), deferred()];
  const started = [deferred(), deferred()];
  const calls = [];
  const queue = createCacheSyncQueue(async (keys, stale) => {
    const index = calls.length;
    calls.push({keys, stale});
    started[index].resolve();
    await gates[index].promise;
  });
  const groups = [];
  let enqueueMs = 0;
  for (let batch = 0; batch < 2; batch++) {
    if (batch) await started[0].promise;
    const requests = [];
    const start = performance.now();
    for (let index = 0; index < requestsPerBurst; index++) {
      requests.push(queue(["live_session", index % 2 ? "schedule" : "todo"], index !== requestsPerBurst - 1));
    }
    enqueueMs += performance.now() - start;
    groups.push(requests);
  }
  gates[0].resolve();
  await started[1].promise;
  await groups[0][0];
  gates[1].resolve();
  await groups[1][0];
  // Every return belongs to its own captured batch. Neither history nor keys is truncated.
  await Promise.all(groups.flat());
  assert.deepEqual(calls, [
    {keys:["live_session", "todo", "schedule"], stale:false},
    {keys:["live_session", "todo", "schedule"], stale:false},
  ]);
  return {enqueueMs, returnedCompletions:groups.map(group => new Set(group).size), calls};
}

const results = [];
for (const count of [200, 10000]) {
  const times = [[], []];
  const samples = [];
  for (let sample = -3; sample < 9; sample++) {
    for (const index of sample % 2 ? [1,0] : [0,1]) {
      const result = await run(implementations[index].createCacheSyncQueue, count);
      if (sample >= 0) times[index].push(result.enqueueMs);
      samples[index] = result;
    }
  }
  assert.deepEqual(samples[0].calls, samples[1].calls);
  assert.deepEqual(samples[0].returnedCompletions, [count, count]);
  assert.deepEqual(samples[1].returnedCompletions, [1, 1]);
  results.push({requestsPerBurst:count, bursts:2,
    enqueueMedianMs:{before:median(times[0]), current:median(times[1])},
    distinctReturnedCompletions:{before:samples[0].returnedCompletions, current:samples[1].returnedCompletions},
    sameKeysForcedRefreshAndReadCount:true});
}
console.log(JSON.stringify({results,
  scope:"Node synthetic enqueue timing only, nine alternating medians after three warmups. Includes request arrays and queue bookkeeping; excludes drain timing, consumer promise reactions, native IPC, data reads, DOM, app RSS and GPU. Distinct returned completions are identity counts, not measured heap allocations."}, null, 2));
