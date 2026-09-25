import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/posterLoadQueue.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
});
function setup() {
  const pending = [];
  const exports = {};
  new Function('require', 'exports', outputText)(() => ({
    convertFileSrc: (path) => `asset:${path}`,
    invoke: (_, args) => new Promise((resolve, reject) => pending.push({ args, resolve, reject })),
  }), exports);
  return { ...exports, pending };
}
const tick = () => new Promise((resolve) => setImmediate(resolve));
const opts = { folderPath: '/movies/A' };

test('missing and failed thumbnails are retryable; successful requests are shared', async () => {
  const api = setup();
  const first = api.resolvePosterSrc(opts);
  assert.equal(api.resolvePosterSrc(opts), first);
  await tick();
  api.pending[0].resolve(null);
  assert.equal(await first, null);
  const retry = api.resolvePosterSrc(opts);
  await tick();
  api.pending[1].reject(new Error('temporary failure'));
  assert.equal(await retry, null);
  const success = api.resolvePosterSrc(opts);
  await tick();
  api.pending[2].resolve('/thumb/new');
  assert.equal(await success, 'asset:/thumb/new');
  assert.equal(api.resolvePosterSrc(opts), success);
});

test('invalidation discards an in-flight response without evicting its replacement', async () => {
  const api = setup();
  let changes = 0;
  const unsubscribe = api.subscribePosterCache(() => changes++);
  const old = api.resolvePosterSrc(opts);
  await tick();
  api.invalidatePosterCache();
  const fresh = api.resolvePosterSrc(opts);
  await tick();
  api.pending[0].resolve('/thumb/old');
  assert.equal(await old, null);
  assert.equal(api.resolvePosterSrc(opts), fresh);
  api.pending[1].resolve('/thumb/new');
  assert.equal(await fresh, 'asset:/thumb/new');
  assert.equal(changes, 1);
  unsubscribe();
  api.invalidatePosterCache();
  assert.equal(changes, 1);
});

test('limits concurrent IPC and skips invalidated waiting requests', async () => {
  const api = setup();
  const requests = Array.from({ length: 8 }, (_, i) => api.resolvePosterSrc({ folderPath: `/movies/${i}` }));
  await tick();
  assert.equal(api.pending.length, 6);
  api.invalidatePosterCache();
  api.pending.forEach((request) => request.resolve('/thumb/old'));
  assert.deepEqual(await Promise.all(requests), Array(8).fill(null));
  assert.equal(api.pending.length, 6);
});

test('evicts the least recently used entry when the cache reaches capacity', async () => {
  const api = setup();
  for (let i = 0; i < 512; i++) {
    const request = api.resolvePosterSrc({ folderPath: `/movies/${i}` });
    await tick();
    api.pending.at(-1).resolve(`/thumb/${i}`);
    await request;
  }
  await api.resolvePosterSrc({ folderPath: '/movies/0' });
  const extra = api.resolvePosterSrc({ folderPath: '/movies/512' });
  await tick();
  api.pending.at(-1).resolve('/thumb/512');
  await extra;
  await api.resolvePosterSrc({ folderPath: '/movies/0' });
  assert.equal(api.pending.length, 513);
  const evicted = api.resolvePosterSrc({ folderPath: '/movies/1' });
  await tick();
  assert.equal(api.pending.length, 514);
  api.pending.at(-1).resolve('/thumb/1');
  await evicted;
});
