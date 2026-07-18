import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/editorErrorState.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { reduceEditorError } = await import(moduleUrl)

let error = reduceEditorError(undefined, { type: 'commandError', message: 'build failed' })
for (let frame = 0; frame < 12; frame += 1) error = reduceEditorError(error, { type: 'snapshot' })
assert.equal(error, 'build failed', 'ordinary frame snapshots must not erase a command error')
assert.equal(reduceEditorError(error, { type: 'dismissed' }), undefined)
assert.equal(reduceEditorError(error, { type: 'reconnectSucceeded' }), undefined)

console.log('Editor error state: snapshots preserve errors until explicit resolution.')
