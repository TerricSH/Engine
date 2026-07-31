import { createHash } from 'node:crypto'
import { readdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const editorRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const sandboxRoot = resolve(editorRoot, '..')

const sourceFiles = [
  'editor-web/index.html',
  'editor-web/package.json',
  'editor-web/pnpm-lock.yaml',
  'editor-web/pnpm-workspace.yaml',
  'editor-web/tsconfig.app.json',
  'editor-web/tsconfig.json',
  'editor-web/tsconfig.node.json',
  'editor-web/vite.config.ts',
  'src/editor_app/dispatch.rs',
  'src/editor_app/protocol.rs',
  'src/editor_app/snapshot.rs',
]

async function collectFiles(directory) {
  const entries = await readdir(resolve(sandboxRoot, directory), { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const child = `${directory}/${entry.name}`
    if (entry.isDirectory()) files.push(...await collectFiles(child))
    else if (entry.isFile()) files.push(child)
  }
  return files
}

sourceFiles.push(...await collectFiles('editor-web/scripts'))
sourceFiles.push(...await collectFiles('editor-web/src'))
sourceFiles.push(...await collectFiles('src/editor_app/dispatch'))
sourceFiles.push(...await collectFiles('src/editor_app/protocol'))
sourceFiles.push(...await collectFiles('src/editor_app/snapshot'))
const canonicalInputs = [...new Set(sourceFiles)].sort()
const sourceHash = createHash('sha256')
for (const path of canonicalInputs) {
  sourceHash.update(path)
  sourceHash.update('\0')
  sourceHash.update(await readFile(resolve(sandboxRoot, path)))
  sourceHash.update('\0')
}

const assets = {}
for (const path of ['index.html', 'assets/editor.js', 'assets/editor.css']) {
  assets[path] = createHash('sha256')
    .update(await readFile(resolve(editorRoot, 'dist', path)))
    .digest('hex')
}

const manifest = {
  schema: 1,
  sourceHash: sourceHash.digest('hex'),
  assets,
}
await writeFile(
  resolve(editorRoot, 'dist/build-manifest.json'),
  `${JSON.stringify(manifest, null, 2)}\n`,
)

console.log(`Editor dist manifest: ${relative(process.cwd(), resolve(editorRoot, 'dist/build-manifest.json'))}`)
