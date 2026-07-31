import { readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, extname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const rustProtocol = readFileSync(resolve(webRoot, '../src/editor_app/protocol.rs'), 'utf8')
const rustDispatch = readFileSync(
  resolve(webRoot, '../src/editor_app/dispatch/router.rs'),
  'utf8',
)
const tsProtocol = readFileSync(resolve(webRoot, 'src/bridge/protocol.ts'), 'utf8')
const dockLayout = readFileSync(resolve(webRoot, 'src/layout/dockLayout.ts'), 'utf8')

const decodeStart = rustProtocol.indexOf('impl EditorRequest {')
const decodeEnd = rustProtocol.indexOf('\n}\n\nfn params', decodeStart)
if (decodeStart < 0 || decodeEnd < 0) throw new Error('Could not locate EditorRequest::decode')
const decodeBody = rustProtocol.slice(decodeStart, decodeEnd)
const rustMethods = new Set(
  [...decodeBody.matchAll(/^\s*"([^"]+)"\s*=>/gm)].map((match) => match[1]),
)
const decodedVariants = [...decodeBody.matchAll(/^\s*"([^"]+)"\s*=>\s*Self::([A-Za-z0-9_]+)/gm)]
const variantToMethods = new Map()
for (const [, method, variant] of decodedVariants) {
  variantToMethods.set(variant, [...(variantToMethods.get(variant) ?? []), method])
}
const aliasedVariants = [...variantToMethods.entries()].filter(([, methods]) => methods.length !== 1)
if (decodedVariants.length !== rustMethods.size || aliasedVariants.length) {
  throw new Error(`Each editor method must decode to one unique production request variant; aliases found: ${JSON.stringify(aliasedVariants)}`)
}

const dispatchStart = rustDispatch.indexOf('fn dispatch_editor_request')
if (dispatchStart < 0) throw new Error('Could not locate dispatch_editor_request')
const dispatchBody = rustDispatch.slice(dispatchStart)
const dispatchedVariants = new Set(
  [...dispatchBody.matchAll(/EditorRequest::([A-Za-z0-9_]+)/g)].map((match) => match[1]),
)
const decodedVariantNames = new Set(decodedVariants.map((match) => match[2]))
const missingDispatch = [...decodedVariantNames].filter((variant) => !dispatchedVariants.has(variant)).sort()
const dispatchWithoutDecode = [...dispatchedVariants].filter((variant) => !decodedVariantNames.has(variant)).sort()
if (missingDispatch.length || dispatchWithoutDecode.length) {
  throw new Error([
    'Rust editor decode/dispatch coverage differs.',
    `Decoded without dispatch: ${missingDispatch.join(', ') || '(none)'}`,
    `Dispatched without decode: ${dispatchWithoutDecode.join(', ') || '(none)'}`,
  ].join('\n'))
}

const mapStart = tsProtocol.indexOf('export interface EditorCommandMap {')
const mapEnd = tsProtocol.indexOf('\n}\n\nexport type EditorCommand', mapStart)
if (mapStart < 0 || mapEnd < 0) throw new Error('Could not locate EditorCommandMap')
const commandMapBody = tsProtocol.slice(mapStart, mapEnd)
const tsMethods = new Set(
  [...commandMapBody.matchAll(/^\s*'([^']+)'\s*:/gm)].map((match) => match[1]),
)

const onlyInRust = [...rustMethods].filter((method) => !tsMethods.has(method)).sort()
const onlyInTypeScript = [...tsMethods].filter((method) => !rustMethods.has(method)).sort()
if (onlyInRust.length || onlyInTypeScript.length) {
  throw new Error([
    'React/Rust editor command catalogs differ.',
    `Only in Rust: ${onlyInRust.join(', ') || '(none)'}`,
    `Only in TypeScript: ${onlyInTypeScript.join(', ') || '(none)'}`,
  ].join('\n'))
}

const rustEvents = new Set(
  [...rustProtocol.matchAll(/^pub const [A-Z_]+_EVENT: &str = "([^"]+)";/gm)].map((match) => match[1]),
)
const eventMapStart = tsProtocol.indexOf('export interface EditorEventMap {')
const eventMapEnd = tsProtocol.indexOf('\n}', eventMapStart)
if (eventMapStart < 0 || eventMapEnd < 0) throw new Error('Could not locate EditorEventMap')
const tsEvents = new Set(
  [...tsProtocol.slice(eventMapStart, eventMapEnd).matchAll(/^\s*'([^']+)'\s*:/gm)].map((match) => match[1]),
)
const eventsOnlyInRust = [...rustEvents].filter((event) => !tsEvents.has(event)).sort()
const eventsOnlyInTypeScript = [...tsEvents].filter((event) => !rustEvents.has(event)).sort()
if (eventsOnlyInRust.length || eventsOnlyInTypeScript.length) {
  throw new Error([
    'React/Rust editor event catalogs differ.',
    `Only in Rust: ${eventsOnlyInRust.join(', ') || '(none)'}`,
    `Only in TypeScript: ${eventsOnlyInTypeScript.join(', ') || '(none)'}`,
  ].join('\n'))
}

if (/\blocalStorage\b/.test(dockLayout)) {
  throw new Error('Dock layout must be restored from the project workspace snapshot, not global localStorage.')
}
const workspaceStart = tsProtocol.indexOf('export interface WorkspaceSnapshot {')
const workspaceEnd = tsProtocol.indexOf('\n}', workspaceStart)
const workspaceBody = tsProtocol.slice(workspaceStart, workspaceEnd)
if (!/\breactLayout:\s*string\b/.test(workspaceBody)
  || /\b(?:bottomPanel|showHierarchy|showInspector|showBottomPanel|hierarchyWidth|inspectorWidth|bottomHeight|viewportRect)\b/.test(workspaceBody)) {
  throw new Error('WorkspaceSnapshot must expose only the project-persisted React layout.')
}

function sourceFiles(directory) {
  return readdirSync(directory)
    .map((entry) => resolve(directory, entry))
    .flatMap((entry) => statSync(entry).isDirectory() ? sourceFiles(entry) : [entry])
    .filter((entry) => ['.ts', '.tsx'].includes(extname(entry)))
}

const unknownCalls = []
const calledMethods = new Set()
for (const file of sourceFiles(resolve(webRoot, 'src'))) {
  const source = readFileSync(file, 'utf8')
  for (const match of source.matchAll(/\b(?:invoke|notify)\(\s*['"]([^'"]+)['"]/g)) {
    calledMethods.add(match[1])
    if (!tsMethods.has(match[1])) unknownCalls.push(`${file}: ${match[1]}`)
  }
}
if (unknownCalls.length) {
  throw new Error(`UI calls methods absent from EditorCommandMap:\n${unknownCalls.join('\n')}`)
}
const commandsWithoutUiEntryPoint = [...tsMethods].filter((method) => !calledMethods.has(method)).sort()
if (commandsWithoutUiEntryPoint.length) {
  throw new Error(`Editor commands without a literal React UI entry point:\n${commandsWithoutUiEntryPoint.join('\n')}`)
}

console.log(`Editor IPC contract: ${tsMethods.size} commands and ${tsEvents.size} events, with complete React entry-point coverage.`)
