import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, extname, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const sourceRoot = resolve(webRoot, 'src')

function sourceFiles(directory) {
  return readdirSync(directory)
    .map((entry) => resolve(directory, entry))
    .flatMap((entry) => statSync(entry).isDirectory() ? sourceFiles(entry) : [entry])
    .filter((entry) => ['.ts', '.tsx'].includes(extname(entry)))
}

function attribute(node, name) {
  return node.attributes.properties.find((property) => ts.isJsxAttribute(property) && property.name.text === name)
}

const disconnected = []
for (const file of sourceFiles(sourceRoot)) {
  const source = readFileSync(file, 'utf8')
  const parsed = ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX)
  const visit = (node) => {
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      const tag = node.tagName.getText(parsed)
      const position = parsed.getLineAndCharacterOfPosition(node.getStart(parsed))
      const location = `${relative(webRoot, file)}:${position.line + 1}`
      if (tag === 'button') {
        const type = attribute(node, 'type')
        const submit = type?.initializer && ts.isStringLiteral(type.initializer) && type.initializer.text === 'submit'
        if (!submit && !attribute(node, 'onClick')) disconnected.push(`${location} button has no click handler`)
        const disabled = attribute(node, 'disabled')
        if (disabled && !disabled.initializer) disconnected.push(`${location} button is permanently disabled`)
      }
      if (tag === 'select' && !attribute(node, 'onChange')) disconnected.push(`${location} select has no change handler`)
    }
    ts.forEachChild(node, visit)
  }
  visit(parsed)
}

const menuSource = readFileSync(resolve(sourceRoot, 'components/MenuBar.tsx'), 'utf8')
const paletteSource = readFileSync(resolve(sourceRoot, 'components/CommandPalette.tsx'), 'utf8')
const appSource = readFileSync(resolve(sourceRoot, 'App.tsx'), 'utf8')
const advertised = new Set([
  ...[...menuSource.matchAll(/\{\s*id:\s*'([^']+)'/g)].map((match) => match[1]),
  ...[...paletteSource.matchAll(/^\s*\['([^']+)'/gm)].map((match) => match[1]),
])
const connectedCommands = new Set([
  ...[...appSource.matchAll(/case\s+'([^']+)'/g)].map((match) => match[1]),
  ...[...appSource.matchAll(/'([^']+)'\s*:\s*\{\s*panel:/g)].map((match) => match[1]),
])
const missingCommands = [...advertised].filter((command) => !connectedCommands.has(command)).sort()
if (missingCommands.length) disconnected.push(`advertised commands without an App handler: ${missingCommands.join(', ')}`)

if (disconnected.length) throw new Error(`Disconnected React controls:\n${disconnected.join('\n')}`)
console.log(`React UI controls: ${advertised.size} advertised commands and all native controls are connected.`)
