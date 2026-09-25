import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { test } from 'node:test';
import ts from 'typescript';

const source = readFileSync(new URL('../src/lib/mediaList.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 },
});
const exports = {};
new Function('exports', outputText)(exports);
const { filterAndSortMedia, SORT_OPTIONS } = exports;

// Reference: the previous comparator (folds titles inside every comparison).
const titleKey = (title) => title.normalize('NFKD').replace(/\p{M}/gu, '').toLocaleLowerCase();
const compareTitle = (a, b) => titleKey(a).localeCompare(titleKey(b), undefined, { numeric: true });
const rank = { unscraped: 0, partial: 1, unmatched: 2, scraped: 3 };
function reference(items, option) {
  return [...items].sort((lhs, rhs) => {
    switch (option) {
      case 'nameAscending': return compareTitle(lhs.title, rhs.title);
      case 'nameDescending': return compareTitle(rhs.title, lhs.title);
      case 'yearDescending': {
        const ly = lhs.year ?? Number.MIN_SAFE_INTEGER; const ry = rhs.year ?? Number.MIN_SAFE_INTEGER;
        return ly === ry ? compareTitle(lhs.title, rhs.title) : ry - ly;
      }
      case 'yearAscending': {
        const ly = lhs.year ?? Number.MAX_SAFE_INTEGER; const ry = rhs.year ?? Number.MAX_SAFE_INTEGER;
        return ly === ry ? compareTitle(lhs.title, rhs.title) : ly - ry;
      }
      case 'addedAtDescending':
        return lhs.addedAt === rhs.addedAt ? compareTitle(lhs.title, rhs.title) : lhs.addedAt < rhs.addedAt ? 1 : -1;
      case 'addedAtAscending':
        return lhs.addedAt === rhs.addedAt ? compareTitle(lhs.title, rhs.title) : lhs.addedAt > rhs.addedAt ? 1 : -1;
      case 'unscrapedFirst': {
        const lr = rank[lhs.status] ?? 99; const rr = rank[rhs.status] ?? 99;
        if (lr !== rr) return lr - rr;
        if (lhs.addedAt !== rhs.addedAt) return lhs.addedAt < rhs.addedAt ? 1 : -1;
        return compareTitle(lhs.title, rhs.title);
      }
    }
  });
}

const titles = ['Movie 10', 'Movie 2', 'movie 1', 'Émile', 'Emile', 'Zoë', 'zoe', 'Ａｌｐｈａ', 'alpha', '千と千尋の神隠し', '三体', 'Ōkami', 'Okami 2', 'The Matrix'];
const statuses = ['unscraped', 'partial', 'unmatched', 'scraped', 'weird'];
const items = Array.from({ length: 400 }, (_, i) => ({
  id: `id-${i}`,
  title: `${titles[i % titles.length]}${i % 3 === 0 ? '' : ` ${i % 17}`}`,
  originalTitle: i % 5 === 0 ? `Original ${i}` : null,
  year: i % 7 === 0 ? null : 1990 + (i % 13),
  status: statuses[i % statuses.length],
  addedAt: `2026-01-${String(1 + (i % 9)).padStart(2, '0')}T00:00:00Z`,
}));

test('sort results are identical to the per-comparison reference for every option', () => {
  for (const { value } of SORT_OPTIONS) {
    const got = filterAndSortMedia(items, '', 'all', value).map((m) => m.id);
    const want = reference(items, value).map((m) => m.id);
    assert.deepEqual(got, want, value);
  }
});

test('filters by query and status before sorting', () => {
  const got = filterAndSortMedia(items, '  MOVIE ', 'scraped', 'nameAscending');
  assert.ok(got.length > 0);
  assert.ok(got.every((m) => m.status === 'scraped' && m.title.toLowerCase().includes('movie')));
  const byOriginal = filterAndSortMedia(items, 'original 10', 'all', 'nameAscending');
  assert.ok(byOriginal.some((m) => m.id === 'id-10'));
  assert.notEqual(filterAndSortMedia(items, '', 'all', 'nameAscending'), items);
});
