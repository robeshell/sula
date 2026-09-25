import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import ts from 'typescript';

const require = createRequire(import.meta.url);
const source = readFileSync(new URL('../src/store/appStore.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
});

function setup() {
  const pending = [];
  const invoke = (command, args) => {
    assert.ok(['list_media_page', 'get_media_detail'].includes(command));
    return new Promise((resolve, reject) => pending.push({ args, resolve, reject }));
  };
  const mocks = {
    '../lib/confirmation': { confirmAction: async () => false },
    '@tauri-apps/api/core': { invoke },
    '@tauri-apps/plugin-dialog': { open: () => { throw new Error('unexpected dialog'); } },
    '../i18n': { default: { t: (key) => key } },
    '../lib/mediaList': { filterAndSortMedia: (items) => items },
    '../lib/localizeMessage': { localizeUserMessage: String },
    '../lib/notify': { notifyTaskDone: () => {} },
    '../lib/posterLoadQueue': { POSTER_THUMB: { width: 140, height: 210 }, resolvePosterSrc: async () => null, invalidatePosterCache: () => {} },
  };
  const exports = {};
  new Function('require', 'exports', outputText)(
    (id) => id in mocks ? mocks[id] : require(id), exports,
  );
  return { store: exports.useAppStore, pending };
}
const page = (id) => ({ items: [{ id }], metadata: [], showStats: [] });

test('late response from previous library cannot overwrite the active library', async () => {
  const { store, pending } = setup();
  const first = store.getState().selectLibrary('A');
  const second = store.getState().selectLibrary('B');
  pending[1].resolve(page('B-movie'));
  await second;
  pending[0].resolve(page('A-movie'));
  await first;
  assert.equal(store.getState().selectedLibraryId, 'B');
  assert.equal(store.getState().mediaItems[0].id, 'B-movie');
});

test('latest refresh wins even when requests target the same library', async () => {
  const { store, pending } = setup();
  const first = store.getState().selectLibrary('A');
  const second = store.getState().selectLibrary('A');
  pending[1].resolve(page('new'));
  await second;
  pending[0].resolve(page('old'));
  await first;
  assert.equal(store.getState().mediaItems[0].id, 'new');
});

test('deselecting a library invalidates outstanding errors and clears old rows', async () => {
  const { store, pending } = setup();
  store.setState({ mediaItems: [{ id: 'old' }] });
  const first = store.getState().selectLibrary('A');
  assert.deepEqual(store.getState().mediaItems, []);
  await store.getState().selectLibrary(null);
  pending[0].reject(new Error('stale failure'));
  await first;
  assert.equal(store.getState().error, null);
  assert.equal(store.getState().toastMessage, null);
});


test('pages append progressively and stale remaining pages cannot leak into a new library', async () => {
  const { store, pending } = setup();
  const load = store.getState().selectLibrary('A');
  pending[0].resolve({ ...page('one'), nextOffset: 256 });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(store.getState().mediaItems.length, 1);
  assert.equal(pending[1].args.offset, 256);
  const other = store.getState().selectLibrary('B');
  pending[2].resolve(page('new')); await other;
  pending[1].resolve(page('old-second-page')); await load;
  assert.deepEqual(store.getState().mediaItems.map(x => x.id), ['new']);
});


test('A to B to A detail selection ignores the first A response', async () => {
  const { store, pending } = setup();
  const first = store.getState().selectMedia('A');
  const second = store.getState().selectMedia('B');
  const third = store.getState().selectMedia('A');
  pending[2].resolve({ item: { id: 'A', title: 'latest' } }); await third;
  pending[0].resolve({ item: { id: 'A', title: 'stale' } }); await first;
  pending[1].resolve({ item: { id: 'B' } }); await second;
  assert.equal(store.getState().detail.item.title, 'latest');
});
