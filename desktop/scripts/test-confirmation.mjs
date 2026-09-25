import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { test } from 'node:test';
import ts from 'typescript';
const require = createRequire(import.meta.url);
const source = readFileSync(new URL('../src/lib/confirmation.ts', import.meta.url), 'utf8');
const { outputText } = ts.transpileModule(source, { compilerOptions: { module: ts.ModuleKind.CommonJS, target: ts.ScriptTarget.ES2020 } });
function setup() {
  const exports = {};
  new Function('require', 'exports', outputText)(require, exports);
  return exports;
}
test('destructive confirmation waits for explicit acceptance and resolves once', async () => {
  const { confirmAction, resolveConfirmation, useConfirmation } = setup();
  let resolved = false;
  const result = confirmAction({ title: 'Remove library', description: 'Fixture only' }).then(value => { resolved = true; return value; });
  await Promise.resolve(); assert.equal(resolved, false);
  resolveConfirmation(true); resolveConfirmation(false);
  assert.equal(await result, true); assert.equal(useConfirmation.getState().request, null);
});
test('replacement and dismissal cancel the pending operation', async () => {
  const { confirmAction, resolveConfirmation } = setup();
  const first = confirmAction({ title: 'first', description: '' });
  const second = confirmAction({ title: 'second', description: '' });
  assert.equal(await first, false);
  resolveConfirmation(false); assert.equal(await second, false);
});
