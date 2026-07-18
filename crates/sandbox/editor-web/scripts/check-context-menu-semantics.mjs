import { readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import ts from 'typescript'

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')

function readSource(relativePath) {
  const file = resolve(webRoot, relativePath)
  const source = readFileSync(file, 'utf8')
  return {
    file,
    source,
    parsed: ts.createSourceFile(file, source, ts.ScriptTarget.Latest, true, ts.ScriptKind.TSX),
  }
}

function invariant(condition, message) {
  if (!condition) throw new Error(`Editor context-menu contract failed: ${message}`)
}

function compact(text) {
  return text.replace(/\s+/g, '')
}

function propertyName(property) {
  return property.name && (ts.isIdentifier(property.name) || ts.isStringLiteral(property.name))
    ? property.name.text
    : undefined
}

function property(object, name) {
  return object.properties.find((entry) => ts.isPropertyAssignment(entry) && propertyName(entry) === name)
}

function objectById(document, id) {
  let found
  const visit = (node) => {
    if (found) return
    if (ts.isObjectLiteralExpression(node)) {
      const idProperty = property(node, 'id')
      if (idProperty && ts.isStringLiteral(idProperty.initializer) && idProperty.initializer.text === id) {
        found = node
        return
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(document.parsed)
  invariant(found, `${document.file} must declare menu item '${id}'`)
  return found
}

function propertyText(document, object, name) {
  const entry = property(object, name)
  invariant(entry, `menu item '${property(object, 'id')?.initializer.getText(document.parsed)}' must declare ${name}`)
  return entry.initializer.getText(document.parsed)
}

function assertActionItems(document, ids) {
  for (const id of ids) {
    const object = objectById(document, id)
    invariant(property(object, 'onSelect') || property(object, 'children'), `menu item '${id}' must execute a real action or open a real submenu`)
  }
}

function assertAction(document, id, method, requiredFragments = []) {
  const action = compact(propertyText(document, objectById(document, id), 'onSelect'))
  invariant(action.includes(`controller.invoke('${method}'`), `menu item '${id}' must invoke ${method}`)
  for (const fragment of requiredFragments) {
    invariant(action.includes(compact(fragment)), `menu item '${id}' must pass ${fragment}`)
  }
}

function jsxAttribute(document, tagName, classFragment, attributeName) {
  let result
  const visit = (node) => {
    if (result) return
    if (ts.isJsxOpeningElement(node) || ts.isJsxSelfClosingElement(node)) {
      if (node.tagName.getText(document.parsed) === tagName) {
        const classAttribute = node.attributes.properties.find((entry) => ts.isJsxAttribute(entry) && entry.name.text === 'className')
        if (classAttribute?.getText(document.parsed).includes(classFragment)) {
          result = node.attributes.properties.find((entry) => ts.isJsxAttribute(entry) && entry.name.text === attributeName)
          if (result) return
        }
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(document.parsed)
  invariant(result, `${document.file} ${tagName}.${classFragment} must declare ${attributeName}`)
  return result.getText(document.parsed)
}

const contextMenu = readSource('src/components/ContextMenu.tsx')
const hierarchy = readSource('src/panels/HierarchyPanel.tsx')
const inspector = readSource('src/panels/InspectorPanel.tsx')

const contextText = compact(contextMenu.source)
for (const key of ['ArrowDown', 'ArrowUp', 'Home', 'End', 'ArrowRight', 'ArrowLeft', 'Enter', 'Escape', 'Tab']) {
  invariant(contextMenu.source.includes(`case '${key}':`), `ContextMenu must handle ${key}`)
}
for (const fragment of [
  'createPortal(',
  'role="menu"',
  "'menuitemcheckbox'",
  'if (!entry || isSeparator(entry) || entry.disabled) return',
  'disabled={entry.disabled}',
  "document.addEventListener('pointerdown', closeForOutsidePointer, true)",
  "window.addEventListener('blur', onClose)",
  "window.addEventListener('resize', onClose)",
]) {
  invariant(contextMenu.source.includes(fragment), `ContextMenu must retain ${fragment}`)
}
invariant(contextText.includes('event.preventDefault()'), 'opening a context menu must prevent the browser menu')
invariant(contextText.includes('if(options.stopPropagation!==false)event.stopPropagation()'), 'opening a context menu must stop trigger propagation by default')
const disabledGuardIndex = contextText.indexOf('if(!entry||isSeparator(entry)||entry.disabled)return')
const actionDispatchIndex = contextText.indexOf('voidentry.onSelect?.()')
invariant(disabledGuardIndex >= 0 && actionDispatchIndex >= 0 && disabledGuardIndex < actionDispatchIndex, 'disabled entries must be rejected before action dispatch')

const hierarchyActionIds = [
  'create-child', 'create-child-menu', 'cut', 'copy', 'paste-child', 'duplicate', 'rename',
  'delete', 'active', 'move', 'unparent', 'focus', 'create-empty', 'create-menu', 'paste-root',
  'select-all', 'select-prefab', 'unpack-prefab', 'unpack-prefab-complete',
]
assertActionItems(hierarchy, hierarchyActionIds)
assertAction(hierarchy, 'cut', 'scene.cutEntities', ['entityIds:ids'])
assertAction(hierarchy, 'copy', 'scene.copyEntities', ['entityIds:ids'])
assertAction(hierarchy, 'paste-child', 'scene.pasteEntities', ['parentId:node.id'])
assertAction(hierarchy, 'duplicate', 'scene.duplicateEntity', ['entityIds:ids'])
assertAction(hierarchy, 'delete', 'scene.deleteEntity', ['entityIds:ids'])
assertAction(hierarchy, 'active', 'scene.setEntityEnabled', ['entityIds:ids'])
assertAction(hierarchy, 'unparent', 'scene.setEntityParent', ['entityIds:ids'])
assertAction(hierarchy, 'focus', 'viewport.focusSelection')
assertAction(hierarchy, 'unpack-prefab', 'assets.unpackPrefab', ["mode:'instance'"])
assertAction(hierarchy, 'unpack-prefab-complete', 'assets.unpackPrefab', ["mode:'completely'"])

for (const id of ['paste-child', 'paste-root']) {
  const disabled = compact(propertyText(hierarchy, objectById(hierarchy, id), 'disabled'))
  invariant(disabled.includes('!editing'), `${id} must be disabled outside Edit mode`)
  invariant(disabled.includes('project.clipboard.entityRootCount===0'), `${id} must be gated by the entity clipboard`)
}

const rowDrop = compact(jsxAttribute(hierarchy, 'div', 'hierarchy-row', 'onDrop'))
invariant(rowDrop.includes('event.preventDefault()'), 'Hierarchy row drop must prevent the browser default')
invariant(rowDrop.includes('event.stopPropagation()'), 'Hierarchy row drop must not bubble into the root drop target')
invariant(rowDrop.includes('if(!editing)return'), 'Hierarchy row drop must be disabled outside Edit mode')
invariant(rowDrop.includes("controller.invoke('scene.setEntityParent',{entityIds,parent:node.id})"), 'Hierarchy row drop must reparent the complete dragged selection')
invariant(rowDrop.indexOf('event.stopPropagation()') < rowDrop.indexOf("controller.invoke('scene.setEntityParent'"), 'Hierarchy row drop must stop propagation before dispatch')
invariant(compact(jsxAttribute(hierarchy, 'div', 'hierarchy-row', 'draggable')) === 'draggable={editing}', 'Hierarchy dragging must be disabled outside Edit mode')
invariant(compact(hierarchy.source).includes('disabled={!editing}onClick={()=>createEntity()}'), 'Hierarchy create button must be disabled outside Edit mode')

const inspectorActionIds = ['reset', 'copy-component', 'paste-component', 'enabled', 'remove']
assertActionItems(inspector, inspectorActionIds)
assertAction(inspector, 'reset', 'scene.resetComponent', ['entityIds'])
assertAction(inspector, 'copy-component', 'scene.copyComponent', ['entityId:activeEntityId'])
assertAction(inspector, 'paste-component', 'scene.pasteComponent', ['entityIds'])
assertAction(inspector, 'enabled', 'scene.setComponentEnabled', ['entityIds'])
assertAction(inspector, 'remove', 'scene.removeComponent', ['entityIds'])

const inspectorText = compact(inspector.source)
const pasteComponentDisabled = compact(propertyText(inspector, objectById(inspector, 'paste-component'), 'disabled'))
invariant(inspectorText.includes('project.clipboard.componentType===component.typeId'), 'Inspector paste must require an exact component clipboard type')
invariant(pasteComponentDisabled.includes('!editing') && pasteComponentDisabled.includes('!clipboardMatches'), 'Inspector paste must be disabled in Play mode and for incompatible clipboards')
const removeDisabled = compact(propertyText(inspector, objectById(inspector, 'remove'), 'disabled'))
invariant(removeDisabled.includes('!editing') && removeDisabled.includes('!component.removable') && removeDisabled.includes('component.removeBlockedReason'), 'Remove Component must honor edit mode, required components, and reverse dependencies')
const resetDisabled = compact(propertyText(inspector, objectById(inspector, 'reset'), 'disabled'))
invariant(resetDisabled.includes('!editing') && resetDisabled.includes('!component.resettable'), 'Reset must honor edit mode and registered defaults')
invariant((inspector.source.match(/<fieldset disabled=\{!editing\}/g) ?? []).length >= 2, 'Inspector Transform and component fields must be disabled outside Edit mode')
invariant(inspectorText.includes('disabled={!editing||multiSelection}'), 'multi-selection rename must remain disabled')
invariant(inspectorText.includes('!existingTypes.has(entry.typeId)'), 'Add Component must filter components already present on the selection')

console.log(`Unity-style context menu semantics: ${hierarchyActionIds.length + inspectorActionIds.length} real actions, clipboard gates, drag/drop propagation, and Play-mode guards verified.`)
