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
  const posterInvalidations = [];
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
    '../lib/posterLoadQueue': {
      POSTER_THUMB: { width: 140, height: 210 },
      resolvePosterSrc: async () => null,
      invalidatePosterCache: () => { posterInvalidations.push('all'); },
      invalidatePosterFolders: (folders) => { const list = [...folders]; if (list.length) posterInvalidations.push(list.sort()); },
    },
  };
  const exports = {};
  new Function('require', 'exports', outputText)(
    (id) => id in mocks ? mocks[id] : require(id), exports,
  );
  return { store: exports.useAppStore, pending, posterInvalidations };
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

const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
const settle = () => new Promise((resolve) => setImmediate(resolve));
const item = (id, extra = {}) => ({ id, title: id, folderPath: `/lib/${id}`, status: 'scraped', ...extra });
async function loaded(store, pending, items) {
  const load = store.getState().selectLibrary('L');
  pending.at(-1).resolve({ items, metadata: [], showStats: [] });
  await load;
}

test('background reload keeps the list, prunes the selection and refreshes the open detail', async () => {
  const { store, pending } = setup();
  await loaded(store, pending, [item('a'), item('b'), item('c')]);
  store.setState({ selectedMediaIds: ['a', 'b'], selectedMediaId: null });
  const reload = store.getState().reloadLibraryItems('L');
  assert.equal(store.getState().mediaItems.length, 3, 'list is not blanked while reloading');
  await wait(200);
  assert.equal(pending.length, 2);
  pending[1].resolve({ items: [item('a'), item('c')], metadata: [], showStats: [] });
  await reload;
  assert.deepEqual(store.getState().mediaItems.map((m) => m.id), ['a', 'c']);
  assert.deepEqual(store.getState().selectedMediaIds, ['a']);

  const open = store.getState().selectMedia('c');
  pending[2].resolve({ item: item('c', { title: 'old detail' }) });
  await open;
  const again = store.getState().reloadLibraryItems('L');
  await wait(200);
  pending[3].resolve({ items: [item('a'), item('c')], metadata: [], showStats: [] });
  await again;
  await settle();
  assert.equal(store.getState().selectedMediaId, 'c');
  assert.equal(store.getState().detailLoading, false);
  assert.equal(pending[4].args.id, 'c');
  pending[4].resolve({ item: item('c', { title: 'new detail' }) });
  await settle();
  assert.equal(store.getState().detail.item.title, 'new detail');
});

test('task completion and library-updated coalesce into one reload', async () => {
  const { store, pending } = setup();
  await loaded(store, pending, [item('a')]);
  const first = store.getState().reloadLibraryItems('L');
  const second = store.getState().reloadLibraryItems('L', { posters: 'all' });
  await wait(200);
  assert.equal(pending.length, 2);
  pending[1].resolve({ items: [item('a')], metadata: [], showStats: [] });
  await Promise.all([first, second]);
  assert.equal(pending.length, 2);
});

test('a reload requested mid-flight runs once more afterwards', async () => {
  const { store, pending } = setup();
  await loaded(store, pending, [item('a')]);
  const first = store.getState().reloadLibraryItems('L');
  await wait(200);
  assert.equal(pending.length, 2);
  const second = store.getState().reloadLibraryItems('L');
  const third = store.getState().reloadLibraryItems('L');
  pending[1].resolve({ items: [item('a')], metadata: [], showStats: [] });
  await first;
  await wait(200);
  assert.equal(pending.length, 3);
  pending[2].resolve({ items: [item('a'), item('b')], metadata: [], showStats: [] });
  await Promise.all([second, third]);
  assert.deepEqual(store.getState().mediaItems.map((m) => m.id), ['a', 'b']);
});

test('reload invalidates only posters whose item or metadata changed', async () => {
  const { store, pending, posterInvalidations } = setup();
  await loaded(store, pending, [item('a'), item('b')]);
  const reload = store.getState().reloadLibraryItems('L');
  await wait(200);
  pending[1].resolve({
    items: [item('a'), item('b')],
    metadata: [{ mediaItemId: 'b', posterPath: 'poster.jpg', genres: [] }],
    showStats: [],
  });
  await reload;
  assert.deepEqual(posterInvalidations, [['/lib/b']]);
});

test('switching library drops a pending background reload', async () => {
  const { store, pending } = setup();
  await loaded(store, pending, [item('a')]);
  const reload = store.getState().reloadLibraryItems('L');
  const other = store.getState().selectLibrary('M');
  pending[1].resolve(page('m'));
  await other;
  await wait(200);
  await reload;
  assert.equal(pending.length, 2);
  assert.deepEqual(store.getState().mediaItems.map((m) => m.id), ['m']);
});

test('confirming a delete whose selection vanished tells the user', async () => {
  const { store } = setup();
  await store.getState().deleteSelectedItems(false);
  assert.equal(store.getState().toastMessage, 'toast.deleteSelectionGone');
});
