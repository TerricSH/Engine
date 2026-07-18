import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { AssetSnapshot, ComponentField, ComponentSnapshot, EngineValue, Vec3 } from '../bridge/protocol'
import type { EditorController } from '../state/useEditorState'
import { ContextMenu, type ContextMenuEntry, useContextMenu } from '../components/ContextMenu'
import { Icon } from '../components/Icon'

function vectorObject(value: unknown, labels: string[]): Record<string, number> {
  if (Array.isArray(value)) return Object.fromEntries(labels.map((axis, index) => [axis, Number(value[index] ?? 0)]))
  const record = typeof value === 'object' && value !== null ? value as Record<string, unknown> : {}
  return Object.fromEntries(labels.map((axis) => [axis, Number(record[axis] ?? 0)]))
}

function useSyncedDraft<T>(identity: string, source: T): {
  draft: T
  setDraft(value: T): void
  beginEditing(): void
  finishEditing(): void
  reset(): void
} {
  const [draft, setDraft] = useState(source)
  const identityRef = useRef(identity)
  const editingRef = useRef(false)
  const sourceRef = useRef(source)
  const sourceFingerprint = JSON.stringify(source)

  useLayoutEffect(() => {
    sourceRef.current = source
    if (identityRef.current !== identity) {
      identityRef.current = identity
      editingRef.current = false
      setDraft(source)
    } else if (!editingRef.current) {
      setDraft(source)
    }
  }, [identity, sourceFingerprint])

  return {
    draft,
    setDraft,
    beginEditing() { editingRef.current = true },
    finishEditing() { editingRef.current = false },
    reset() { editingRef.current = false; setDraft(sourceRef.current) },
  }
}

function VectorEditor({ identity, value, labels, onCommit }: { identity: string; value: unknown; labels: string[]; onCommit(value: Record<string, number>): void }) {
  const source = vectorObject(value, labels)
  const sourceText = Object.fromEntries(labels.map((axis) => [axis, String(source[axis] ?? 0)]))
  const editor = useSyncedDraft(identity, sourceText)
  return <div className="vector-editor">{labels.map((axis) => <label key={axis} className={`axis-${axis}`}><span>{axis.toUpperCase()}</span><input
    type="number"
    value={editor.draft[axis] ?? ''}
    step="any"
    onFocus={editor.beginEditing}
    onChange={(event) => editor.setDraft({ ...editor.draft, [axis]: event.target.value })}
    onBlur={(event) => {
      const next = { ...editor.draft, [axis]: event.target.value }
      const numeric = Object.fromEntries(labels.map((label) => [label, Number(next[label])]))
      editor.finishEditing()
      if (Object.values(numeric).every(Number.isFinite)) onCommit(numeric)
      else editor.reset()
    }}
  /></label>)}</div>
}

function replaceEngineValue(field: ComponentField, value: unknown): EngineValue | undefined {
  const original = field.engineValue
  if ('Bool' in original) return { Bool: Boolean(value) }
  if ('Int' in original || 'UInt' in original) {
    const integer = Number(value)
    if (!Number.isSafeInteger(integer) || ('UInt' in original && integer < 0)) return undefined
    return 'Int' in original ? { Int: integer } : { UInt: integer }
  }
  if ('Float32' in original) { const number = Number(value); return Number.isFinite(number) ? { Float32: number } : undefined }
  if ('Float64' in original) { const number = Number(value); return Number.isFinite(number) ? { Float64: number } : undefined }
  if ('Str' in original) return { Str: String(value) }
  if ('Enum' in original) return { Enum: String(value) }
  if ('Entity' in original) return { Entity: String(value) }
  if ('Vec3' in original) { const v = vectorObject(value, ['x', 'y', 'z']); return { Vec3: [v.x ?? 0, v.y ?? 0, v.z ?? 0] } }
  if ('Quat' in original) { const v = vectorObject(value, ['x', 'y', 'z', 'w']); return { Quat: [v.x ?? 0, v.y ?? 0, v.z ?? 0, v.w ?? 1] } }
  if ('Color' in original) { const v = vectorObject(value, ['x', 'y', 'z', 'w']); return { Color: [v.x ?? 0, v.y ?? 0, v.z ?? 0, v.w ?? 1] } }
  return undefined
}

function AssetReferenceEditor({ field, assets, onCommit }: { field: ComponentField; assets: AssetSnapshot[]; onCommit(value: EngineValue): void }) {
  if (!('Asset' in field.engineValue)) return <span className="readonly-value">Invalid asset reference</span>
  const current = field.engineValue.Asset
  const assetKey = (asset: typeof current) => JSON.stringify([asset.id, asset.logical_path])
  const currentKey = current.id ? assetKey(current) : ''
  if (field.acceptedAssetKinds.length === 0) return <span className="readonly-value" title="This component has not declared a compatible asset type">{current.id || 'No asset'} · type metadata required</span>
  const candidates = assets.filter((asset) => field.acceptedAssetKinds.includes(asset.kind))
  const currentInCatalog = candidates.some((asset) => assetKey(asset.assetId) === currentKey)
  return <div className="object-reference-select"><Icon name="asset" /><select value={currentKey} aria-label={`${field.label} asset`} onChange={(event) => {
    const selected = candidates.find((asset) => assetKey(asset.assetId) === event.target.value)
    if (selected) onCommit({ Asset: selected.assetId })
  }}>
    {!current.id && <option value="">Select an asset…</option>}
    {current.id && !currentInCatalog && <option value={currentKey}>{current.id} (not in compatible project assets)</option>}
    {candidates.map((asset) => <option value={assetKey(asset.assetId)} key={assetKey(asset.assetId)}>{asset.name} · {asset.kind}</option>)}
  </select></div>
}

function ScalarFieldEditor({ identity, field, onCommit }: { identity: string; field: ComponentField; onCommit(value: EngineValue): void }) {
  const integerUnsafe = (('Int' in field.engineValue) || ('UInt' in field.engineValue)) && !Number.isSafeInteger(Number(field.value))
  const source = typeof field.value === 'string' || typeof field.value === 'number' ? String(field.value) : ''
  const editor = useSyncedDraft(identity, source)
  return <input type={field.valueType === 'number' ? 'number' : 'text'} value={editor.draft} readOnly={integerUnsafe} title={integerUnsafe ? 'This 64-bit integer cannot be edited safely in JavaScript' : undefined} step={field.valueType === 'number' ? 'any' : undefined} onFocus={editor.beginEditing} onChange={(event) => editor.setDraft(event.target.value)} onBlur={(event) => {
    editor.finishEditing()
    const engineValue = replaceEngineValue(field, field.valueType === 'number' ? Number(event.target.value) : event.target.value)
    if (engineValue) onCommit(engineValue)
    else editor.reset()
  }} onKeyDown={(event) => { if (event.key === 'Enter') event.currentTarget.blur() }} />
}

function FieldEditor({ identity, field, assets, onCommit }: { identity: string; field: ComponentField; assets: AssetSnapshot[]; onCommit(value: EngineValue): void }) {
  const commit = (value: unknown) => { const engineValue = replaceEngineValue(field, value); if (engineValue) onCommit(engineValue) }
  if (field.valueType === 'boolean') return <input type="checkbox" checked={Boolean(field.value)} onChange={(event) => commit(event.target.checked)} />
  if (field.valueType === 'vec3') return <VectorEditor identity={identity} value={field.value} labels={['x', 'y', 'z']} onCommit={commit} />
  if (field.valueType === 'vec4' || field.valueType === 'color') return <VectorEditor identity={identity} value={field.value} labels={['x', 'y', 'z', 'w']} onCommit={commit} />
  if (field.valueType === 'asset') return <AssetReferenceEditor field={field} assets={assets} onCommit={onCommit} />
  if (field.valueType === 'list' || field.valueType === 'map') return <div className="structured-readonly" title="Structured values are serialized read-only until a schema-specific editor is registered"><small>{field.valueType === 'list' ? 'Read-only list' : 'Read-only map'}</small><pre>{JSON.stringify(field.value, null, 2)}</pre></div>
  return <ScalarFieldEditor identity={identity} field={field} onCommit={onCommit} />
}

function eulerDegreesToQuat(value: Vec3): [number, number, number, number] {
  const x = value.x * Math.PI / 360; const y = value.y * Math.PI / 360; const z = value.z * Math.PI / 360
  const sx = Math.sin(x); const cx = Math.cos(x); const sy = Math.sin(y); const cy = Math.cos(y); const sz = Math.sin(z); const cz = Math.cos(z)
  return [sx * cy * cz + cx * sy * sz, cx * sy * cz - sx * cy * sz, cx * cy * sz + sx * sy * cz, cx * cy * cz - sx * sy * sz]
}

function useComponentMenu(component: Pick<ComponentSnapshot, 'typeId' | 'displayName' | 'enabled' | 'removable' | 'resettable' | 'removeBlockedReason'>, controller: EditorController) {
  const menu = useContextMenu()
  const project = controller.state.project
  const entityIds = project.selection.entityIds
  const activeEntityId = project.selection.activeEntityId
  const authoringBlockedReason = !project.capabilities.editing
    ? 'Stop Play mode before editing'
    : project.capabilities.buildBusy
      ? 'Wait for the active project operation to finish'
      : undefined
  const editing = !authoringBlockedReason
  const clipboardMatches = project.clipboard.componentType === component.typeId
  const items: ContextMenuEntry[] = [
    { id: 'reset', label: 'Reset', disabled: !editing || !component.resettable, disabledReason: !component.resettable ? 'This component has no registered default factory' : authoringBlockedReason, onSelect: () => void controller.invoke('scene.resetComponent', { entityIds, componentType: component.typeId }) },
    { id: 'copy-component', label: 'Copy Component', disabled: !activeEntityId, onSelect: () => activeEntityId && void controller.invoke('scene.copyComponent', { entityId: activeEntityId, componentType: component.typeId }) },
    { id: 'paste-component', label: 'Paste Component Values', disabled: !editing || !clipboardMatches, disabledReason: !editing ? authoringBlockedReason : project.clipboard.componentType ? `Clipboard contains ${project.clipboard.componentType}` : 'Component clipboard is empty', onSelect: () => void controller.invoke('scene.pasteComponent', { entityIds, componentType: component.typeId }) },
    { type: 'separator', id: 'component-state' },
    { id: 'enabled', label: 'Enabled', checked: component.enabled, disabled: !editing, disabledReason: authoringBlockedReason, onSelect: () => void controller.invoke('scene.setComponentEnabled', { entityIds, componentType: component.typeId, enabled: !component.enabled }) },
    { type: 'separator', id: 'component-remove' },
    { id: 'remove', label: 'Remove Component', danger: true, disabled: !editing || !component.removable || Boolean(component.removeBlockedReason), disabledReason: component.removeBlockedReason ?? (!component.removable ? 'This component is required' : authoringBlockedReason), onSelect: () => void controller.invoke('scene.removeComponent', { entityIds, componentType: component.typeId }) },
  ]
  return { ...menu, items }
}

function TransformEditor({ transform, component, controller }: { transform: NonNullable<EditorController['state']['project']['selection']['transform']>; component: ComponentSnapshot; controller: EditorController }) {
  const entityId = controller.state.project.selection.activeEntityId
  const entityIds = controller.state.project.selection.entityIds
  const editing = controller.state.project.capabilities.editing && !controller.state.project.capabilities.buildBusy
  const menu = useComponentMenu(component, controller)
  const update = (fieldName: string, value: EngineValue) => { if (entityId) void controller.invoke('scene.setComponentField', { entityIds, componentType: 'engine.transform', fieldName, value }) }
  return <div className="component-card transform-card">
    <div className="component-header" onContextMenu={(event) => menu.openContextMenu(event, menu.items, { ariaLabel: 'Transform actions' })}><Icon name="move" /><strong>Transform</strong><span className="component-required-lock" title="Required component"><Icon name="lock" /></span><button type="button" title="Transform component menu" onClick={(event) => { const bounds = event.currentTarget.getBoundingClientRect(); menu.openContextMenu({ clientX: bounds.right, clientY: bounds.bottom, currentTarget: event.currentTarget, preventDefault() {}, stopPropagation() {} }, menu.items, { ariaLabel: 'Transform actions' }) }}><Icon name="menu" /></button></div>
    <fieldset disabled={!editing} className="component-body transform-fields">
      <label><span>Position</span><VectorEditor key={`${entityId}:translation`} identity={`${entityId}:engine.transform:translation`} value={transform.position} labels={['x', 'y', 'z']} onCommit={(value) => update('translation', { Vec3: [value.x ?? 0, value.y ?? 0, value.z ?? 0] })} /></label>
      <label><span>Rotation</span><VectorEditor key={`${entityId}:rotation`} identity={`${entityId}:engine.transform:rotation`} value={transform.rotationEuler} labels={['x', 'y', 'z']} onCommit={(value) => update('rotation', { Quat: eulerDegreesToQuat({ x: value.x ?? 0, y: value.y ?? 0, z: value.z ?? 0 }) })} /></label>
      <label><span>Scale</span><VectorEditor key={`${entityId}:scale`} identity={`${entityId}:engine.transform:scale`} value={transform.scale} labels={['x', 'y', 'z']} onCommit={(value) => update('scale', { Vec3: [value.x ?? 1, value.y ?? 1, value.z ?? 1] })} /></label>
    </fieldset>
    <ContextMenu request={menu.request} onClose={menu.closeContextMenu} />
  </div>
}

function ComponentCard({ component, controller }: { component: ComponentSnapshot; controller: EditorController }) {
  const [expanded, setExpanded] = useState(true)
  const entityId = controller.state.project.selection.activeEntityId
  const entityIds = controller.state.project.selection.entityIds
  const editing = controller.state.project.capabilities.editing && !controller.state.project.capabilities.buildBusy
  const menu = useComponentMenu(component, controller)
  if (!entityId) return null
  return <div className={`component-card ${expanded ? '' : 'collapsed'}`}>
    <div className="component-header" onContextMenu={(event) => menu.openContextMenu(event, menu.items, { ariaLabel: `${component.displayName} actions` })}>
      <button className={`component-expander ${expanded ? 'expanded' : ''}`} type="button" title={expanded ? 'Collapse component' : 'Expand component'} onClick={() => setExpanded(!expanded)}><Icon name="chevron" /></button>
      <input type="checkbox" checked={component.enabled} disabled={!editing} onChange={(event) => void controller.invoke('scene.setComponentEnabled', { entityIds, componentType: component.typeId, enabled: event.target.checked })} />
      <Icon name="cube" /><strong>{component.displayName}</strong>
      {!component.removable && <span className="component-required-lock" title="Required component"><Icon name="lock" /></span>}
      <button type="button" title="Component menu" onClick={(event) => { const bounds = event.currentTarget.getBoundingClientRect(); menu.openContextMenu({ clientX: bounds.right, clientY: bounds.bottom, currentTarget: event.currentTarget, preventDefault() {}, stopPropagation() {} }, menu.items, { ariaLabel: `${component.displayName} actions` }) }}><Icon name="menu" /></button>
    </div>
    {expanded && <fieldset disabled={!editing} className="component-body property-grid">{component.fields.map((field) => <label key={`${entityId}:${field.path}`} title={field.path}><span>{field.label}</span><FieldEditor identity={`${entityId}:${component.typeId}:${field.path}`} field={field} assets={controller.state.project.assets} onCommit={(value) => void controller.invoke('scene.setComponentField', { entityIds, componentType: component.typeId, fieldName: field.path, value })} /></label>)}{component.fields.length === 0 && <span className="component-empty">No editable properties</span>}</fieldset>}
    <ContextMenu request={menu.request} onClose={menu.closeContextMenu} />
  </div>
}

function EntityNameEditor({ entityId, name, disabled, controller }: { entityId: string; name: string; disabled: boolean; controller: EditorController }) {
  const editor = useSyncedDraft(entityId, name)
  const cancelled = useRef(false)
  return <input className="entity-name-input" value={editor.draft} disabled={disabled} onFocus={() => { cancelled.current = false; editor.beginEditing() }} onChange={(event) => editor.setDraft(event.target.value)} onBlur={(event) => {
    editor.finishEditing()
    if (cancelled.current) { cancelled.current = false; editor.reset(); return }
    const next = event.target.value.trim()
    if (!next || next === name) { editor.reset(); return }
    void controller.invoke('scene.renameEntity', { entityId, name: next }).then((result) => { if (!result) editor.reset() })
  }} onKeyDown={(event) => {
    if (event.key === 'Enter') event.currentTarget.blur()
    if (event.key === 'Escape') { cancelled.current = true; event.currentTarget.blur() }
  }} />
}

export function InspectorPanel({ controller }: { controller: EditorController }) {
  const [componentSearch, setComponentSearch] = useState<string>()
  const selection = controller.state.project.selection
  const authoringBlockedReason = !controller.state.project.capabilities.editing
    ? 'Stop Play mode before editing'
    : controller.state.project.capabilities.buildBusy
      ? 'Wait for the active project operation to finish'
      : undefined
  const editing = !authoringBlockedReason
  const existingTypes = useMemo(() => new Set(selection.components.map((component) => component.typeId)), [selection.components])
  const catalog = useMemo(() => {
    const query = componentSearch?.toLocaleLowerCase() ?? ''
    return controller.state.project.catalog.components.filter((entry) => !existingTypes.has(entry.typeId) && `${entry.category} ${entry.displayName}`.toLocaleLowerCase().includes(query))
  }, [componentSearch, controller.state.project.catalog.components, existingTypes])
  const scriptCatalog = useMemo(() => {
    if (selection.entityIds.length !== 1 || existingTypes.has('engine.script')) return []
    const query = componentSearch?.toLocaleLowerCase() ?? ''
    return controller.state.project.catalog.verifiedScriptClasses.filter((entry) => `${entry.assemblyId} ${entry.className}`.toLocaleLowerCase().includes(query))
  }, [componentSearch, controller.state.project.catalog.verifiedScriptClasses, existingTypes, selection.entityIds.length])
  useEffect(() => { const open = () => setComponentSearch(''); window.addEventListener('editor-open-add-component', open); return () => window.removeEventListener('editor-open-add-component', open) }, [])
  if (!selection.activeEntityId) return <div className="panel-column inspector-panel" data-editor-context="inspector"><div className="inspector-tools"><span>Nothing selected</span></div><div className="panel-empty"><Icon name="inspector" /><span>Select a GameObject or asset to inspect it</span></div></div>
  const transformComponent = selection.components.find((component) => component.typeId === 'engine.transform')
  const multiSelection = selection.entityIds.length > 1
  return <div className="panel-column inspector-panel" data-editor-context="inspector">
    <div className="inspector-tools"><span className="selection-path">{multiSelection ? `${selection.entityIds.length} GameObjects selected` : selection.displayName}</span></div>
    <div className="panel-scroll inspector-scroll">
      <div className="entity-header"><div className="entity-title-row"><input type="checkbox" checked={selection.active ?? true} disabled={!editing} onChange={(event) => void controller.invoke('scene.setEntityEnabled', { entityIds: selection.entityIds, enabled: event.target.checked })} /><EntityNameEditor key={selection.activeEntityId} entityId={selection.activeEntityId} name={selection.displayName ?? selection.activeEntityId} disabled={!editing || multiSelection} controller={controller} /></div></div>
      {selection.transform && transformComponent && <TransformEditor transform={selection.transform} component={transformComponent} controller={controller} />}
      {selection.components.filter((component) => component.typeId !== 'engine.transform').map((component) => <ComponentCard key={component.typeId} component={component} controller={controller} />)}
      <button className="add-component-button" type="button" disabled={!editing} title={authoringBlockedReason} onClick={() => setComponentSearch('')}>Add Component</button>
    </div>
    {componentSearch !== undefined && editing && <div className="component-picker"><div className="component-picker-header"><Icon name="search" /><input autoFocus value={componentSearch} placeholder="Search components or scripts" onChange={(event) => setComponentSearch(event.target.value)} /><button type="button" onClick={() => setComponentSearch(undefined)}><Icon name="close" /></button></div><div className="component-picker-list">{catalog.map((entry) => <button type="button" key={entry.typeId} onClick={() => { void controller.invoke('scene.addComponent', { entityIds: selection.entityIds, componentType: entry.typeId }); setComponentSearch(undefined) }}><small>{entry.category}</small><span>{entry.displayName}</span></button>)}{scriptCatalog.map((entry) => <button type="button" key={`${entry.assemblyId}:${entry.className}`} onClick={() => { void controller.invoke('script.attach', { entityId: selection.activeEntityId!, assemblyId: entry.assemblyId, className: entry.className }); setComponentSearch(undefined) }}><small>Scripts · {entry.assemblyId}</small><span>{entry.className}</span></button>)}{catalog.length === 0 && scriptCatalog.length === 0 && <div className="component-picker-empty">No additional registered components or verified scripts match this search.</div>}</div></div>}
  </div>
}
