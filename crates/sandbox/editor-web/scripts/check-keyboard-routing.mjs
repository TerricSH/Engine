import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import { pathToFileURL } from 'node:url'
import ts from 'typescript'

const sourceUrl = new URL('../src/keyboardRouting.ts', import.meta.url)
const source = await readFile(sourceUrl, 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
  fileName: pathToFileURL(sourceUrl.pathname).href,
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { editorShortcutAllowedForViewport } = await import(moduleUrl)

for (const key of ['f', 'F', 'w', 'W', 'e', 'E', 'r', 'R', 'Delete']) {
  assert.equal(
    editorShortcutAllowedForViewport('game', key),
    false,
    `${key} must remain owned by a focused Game viewport`,
  )
}
assert.equal(editorShortcutAllowedForViewport('game', 'F5'), true)
assert.equal(editorShortcutAllowedForViewport('scene', 'W'), true)
assert.equal(editorShortcutAllowedForViewport(undefined, 'Delete'), true)

console.log('Editor keyboard routing: Game viewport input is isolated.')
