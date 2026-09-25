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

test('aborted queued requests never reach IPC and resolve null', async () => {
  const api = setup();
  const controllers = Array.from({ length: 8 }, () => new AbortController());
  const requests = controllers.map((c, i) => api.resolvePosterSrc({ folderPath: `/movies/${i}`, signal: c.signal }));
  await tick();
  assert.equal(api.pending.length, 6);
  controllers[6].abort();
  controllers[7].abort();
  assert.equal(await requests[6], null);
  assert.equal(await requests[7], null);
  api.pending.forEach((request, i) => request.resolve(`/thumb/${i}`));
  await Promise.all(requests);
  await tick();
  assert.equal(api.pending.length, 6);
  // A cancelled key is not cached: asking again issues a real request.
  const again = api.resolvePosterSrc({ folderPath: '/movies/7' });
  await tick();
  assert.equal(api.pending.length, 7);
  api.pending[6].resolve('/thumb/7');
  assert.equal(await again, 'asset:/thumb/7');
});

test('a shared request keeps running while any caller still wants it', async () => {
  const api = setup();
  const fill = Array.from({ length: 6 }, (_, i) => api.resolvePosterSrc({ folderPath: `/fill/${i}` }));
  const a = new AbortController();
  const b = new AbortController();
  const first = api.resolvePosterSrc({ folderPath: '/movies/shared', signal: a.signal });
  const second = api.resolvePosterSrc({ folderPath: '/movies/shared', signal: b.signal });
  assert.equal(first, second);
  a.abort();
  await tick();
  api.pending[0].resolve('/thumb/fill');
  await fill[0];
  await tick();
  assert.equal(api.pending.length, 7);
  assert.equal(api.pending[6].args.folderPath, '/movies/shared');
  api.pending[6].resolve('/thumb/shared');
  assert.equal(await second, 'asset:/thumb/shared');
});

test('queued requests are served newest first', async () => {
  const api = setup();
  for (let i = 0; i < 6; i++) api.resolvePosterSrc({ folderPath: `/fill/${i}` });
  api.resolvePosterSrc({ folderPath: '/movies/old' });
  api.resolvePosterSrc({ folderPath: '/movies/new' });
  await tick();
  api.pending[0].resolve(null);
  await tick(); await tick();
  assert.equal(api.pending.length, 7);
  assert.equal(api.pending[6].args.folderPath, '/movies/new');
});

test('folder invalidation only drops that folder and bumps only its version', async () => {
  const api = setup();
  const a = api.resolvePosterSrc({ folderPath: '/movies/A' });
  const b = api.resolvePosterSrc({ folderPath: '/movies/B' });
  await tick();
  api.pending[0].resolve('/thumb/A');
  api.pending[1].resolve('/thumb/B');
  await Promise.all([a, b]);
  const versionA = api.posterFolderVersion('/movies/A');
  const versionB = api.posterFolderVersion('/movies/B');
  let changes = 0;
  api.subscribePosterCache(() => changes++);
  api.invalidatePosterFolders(['/movies/A']);
  assert.equal(changes, 1);
  assert.notEqual(api.posterFolderVersion('/movies/A'), versionA);
  assert.equal(api.posterFolderVersion('/movies/B'), versionB);
  assert.equal(api.resolvePosterSrc({ folderPath: '/movies/B' }), b);
  const freshA = api.resolvePosterSrc({ folderPath: '/movies/A' });
  assert.notEqual(freshA, a);
  await tick();
  api.pending[2].resolve('/thumb/A2');
  assert.equal(await freshA, 'asset:/thumb/A2');
  api.invalidatePosterFolders([]);
  assert.equal(changes, 1);
});
