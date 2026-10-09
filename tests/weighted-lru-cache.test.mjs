import test from 'node:test';
import assert from 'node:assert/strict';
import { loadTypeScript } from './load-typescript.mjs';
const { WeightedLruCache } = await loadTypeScript('src/lib/weightedLruCache.ts');

test('weighted LRU enforces count and byte budgets and promotes real reads', () => {
  const cache = new WeightedLruCache({ maxEntries: 3, maxBytes: 12 });
  cache.set('a', 'first', 4); cache.set('b', 'second', 4); cache.set('c', 'third', 4);
  assert.equal(cache.get('a'), 'first');
  cache.set('d', 'fourth', 5);
  assert.deepEqual([...cache.keys()], ['a', 'd']);
  assert.equal(cache.estimatedBytes, 9);
  cache.set('e', 'fifth', 1); cache.set('f', 'sixth', 0);
  assert.deepEqual([...cache.keys()], ['d', 'e', 'f']);
  assert.equal(cache.estimatedBytes, 6);
});

test('oversized and invalid values preserve the working set while stale replacements are removed', () => {
  const cache = new WeightedLruCache({ maxEntries: 10, maxBytes: 10 });
  cache.set('a', 'original', 4); cache.set('b', 'keep', 4);
  for (const weight of [11, NaN, Infinity, -1]) assert.equal(cache.set('new', 'full oversized value', weight), false);
  assert.deepEqual([...cache.keys()], ['a', 'b']);
  assert.equal(cache.set('a', 'oversized replacement', 11), false);
  assert.deepEqual([...cache.keys()], ['b']);
  assert.equal(cache.estimatedBytes, 4);
  cache.clear(); assert.equal(cache.size, 0); assert.equal(cache.estimatedBytes, 0);
});

test('null and empty values remain distinguishable from misses and replacements use the new weight', () => {
  const cache = new WeightedLruCache({ maxEntries: 3, maxBytes: 10 });
  cache.set('negative', null, 1); cache.set('empty', '', 0); cache.set('replace', 1, 5);
  assert.equal(cache.get('negative'), null); assert.equal(cache.get('empty'), ''); assert.equal(cache.get('absent'), undefined);
  cache.set('replace', 2, 3); assert.equal(cache.estimatedBytes, 4); assert.equal(cache.get('replace'), 2);
  assert.equal(cache.delete('negative'), true); assert.equal(cache.delete('negative'), false);
  assert.equal(cache.estimatedBytes, 3);
  for (const options of [{ maxEntries: 0 }, { maxBytes: 0 }]) {
    const disabled = new WeightedLruCache(options); assert.equal(disabled.set('no', 'value', 1), false); assert.equal(disabled.size, 0);
  }
});
