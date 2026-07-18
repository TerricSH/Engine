import { useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from 'react'
import type { EntityTemplateSnapshot, HierarchyNode } from '../bridge/protocol'
import type { EditorController } from '../state/useEditorState'
import {
  ContextMenu,
  type ContextMenuEntry,
  type ContextMenuItem,
  useContextMenu,
} from '../components/ContextMenu'
import { Icon } from '../components/Icon'

interface HierarchyPanelProps { controller: EditorController }
interface VisibleNode { node: HierarchyNode; depth: number; parentId?: string }

function filterHierarchy(node: HierarchyNode, query: string): HierarchyNode | undefined {
  const children = node.children.flatMap((child) => {
    const filtered = filterHierarchy(child, query)
    return filtered ? [filtered] : []
  })
  return node.name.toLocaleLowerCase().includes(query) || children.length > 0
    ? { ...node, children }
    : undefined
}

function flattenHierarchy(
  nodes: HierarchyNode[],
  expanded: ReadonlySet<string>,
  filterActive: boolean,
  depth = 0,
  parentId?: string,
): VisibleNode[] {
  const flattened: VisibleNode[] = []
  for (const node of nodes) {
    flattened.push({ node, depth, parentId })
    if (filterActive || expanded.has(node.id)) {
      flattened.push(...flattenHierarchy(node.children, expanded, filterActive, depth + 1, node.id))
    }
  }
  return flattened
}

function groupedTemplates(
  templates: EntityTemplateSnapshot[],
  create: (templateId: string) => void,
): ContextMenuItem[] {
  const categories = new Map<string, EntityTemplateSnapshot[]>()
  for (const template of templates) {
    const entries = categories.get(template.category) ?? []
    entries.push(template)
    categories.set(template.category, entries)
  }
  return [...categories].map(([category, entries]) => ({
    id: `create-category-${category}`,
    label: category,
    children: entries.map((template) => ({
      id: `create-${template.id}`,
      label: template.displayName,
      onSelect: () => create(template.id),
    })),
  }))
}

function parseDraggedEntities(raw: string): string[] {
  if (!raw) return []
  try {
    const parsed = JSON.parse(raw) as unknown
    return Array.isArray(parsed) && parsed.every((id) => typeof id === 'string') ? parsed : []
  } catch {
    return [raw]
  }
}

function RenameEditor({ node, controller, onClose }: {
  node: HierarchyNode
  controller: EditorController
  onClose(): void
}) {
  const [draft, setDraft] = useState(node.name)
  const cancelled = useRef(false)
  const commit = async () => {
    if (cancelled.current) return
    const name = draft.trim()
    if (name && name !== node.name) await controller.invoke('scene.renameEntity', { entityId: node.id, name })
    onClose()
  }
  return <input
    autoFocus
    className="hierarchy-rename"
    value={draft}
    aria-label={`Rename ${node.name}`}
    onChange={(event) => setDraft(event.target.value)}
    onClick={(event) => event.stopPropagation()}
    onBlur={() => void commit()}
    onKeyDown={(event) => {
      event.stopPropagation()
      if (event.key === 'Enter') event.currentTarget.blur()
      if (event.key === 'Escape') { event.preventDefault(); cancelled.current = true; onClose() }
    }}
  />
}

interface HierarchyRowProps {
  item: VisibleNode
  selected: boolean
  expanded: boolean
  editing: boolean
  renaming: boolean
  selectedIds: string[]
  controller: EditorController
  onSelect(node: HierarchyNode, event: MouseEvent): void
  onKeyboard(node: HierarchyNode, event: KeyboardEvent<HTMLDivElement>): void
  onToggle(id: string): void
  onRename(id?: string): void
  onMenu(event: MouseEvent, node: HierarchyNode): void
}

function HierarchyRow({ item, selected, expanded, editing, renaming, selectedIds, controller, onSelect, onKeyboard, onToggle, onRename, onMenu }: HierarchyRowProps) {
  const { node, depth } = item
  return <div
    className={`hierarchy-row ${selected ? 'selected' : ''} ${node.enabled ? '' : 'disabled'}`}
    style={{ paddingInlineStart: `${8 + depth * 15}px` }}
    role="treeitem"
    aria-selected={selected}
    aria-expanded={node.children.length > 0 ? expanded : undefined}
    data-hierarchy-id={node.id}
    tabIndex={selected ? 0 : -1}
    draggable={editing}
    onDragStart={(event) => {
      const dragged = selected ? selectedIds : [node.id]
      event.dataTransfer.setData('application/x-engine-entities', JSON.stringify(dragged))
      event.dataTransfer.effectAllowed = 'move'
    }}
    onDragOver={(event) => { if (editing) { event.preventDefault(); event.dataTransfer.dropEffect = 'move' } }}
    onDrop={(event) => {
      event.preventDefault()
      event.stopPropagation()
      if (!editing) return
      const entityIds = parseDraggedEntities(event.dataTransfer.getData('application/x-engine-entities'))
      if (entityIds.length) void controller.invoke('scene.setEntityParent', { entityIds, parent: node.id })
    }}
    onClick={(event) => onSelect(node, event)}
    onDoubleClick={() => void controller.invoke('viewport.focusSelection', {})}
    onContextMenu={(event) => onMenu(event, node)}
    onKeyDown={(event) => onKeyboard(node, event)}
  >
    <button className={`tree-expander ${node.children.length === 0 ? 'empty' : ''} ${expanded ? 'expanded' : ''}`} type="button" tabIndex={-1} aria-label={expanded ? 'Collapse' : 'Expand'} onClick={(event) => { event.stopPropagation(); if (node.children.length) onToggle(node.id) }}><Icon name="chevron" /></button>
    <Icon className="hierarchy-object-icon" name={node.prefab ? 'asset' : 'cube'} />
    {renaming ? <RenameEditor node={node} controller={controller} onClose={() => onRename()} /> : <span className="hierarchy-name">{node.name}</span>}
    {node.prefab && <span className="prefab-dot" title="Prefab instance" />}
    {!node.enabled && <Icon className="row-visibility" name="eye" />}
  </div>
}

export function HierarchyPanel({ controller }: HierarchyPanelProps) {
  const project = controller.state.project
  const authoringBlockedReason = !project.capabilities.editing
    ? 'Stop Play mode before editing'
    : project.capabilities.buildBusy
      ? 'Wait for the active project operation to finish'
      : undefined
  const editing = !authoringBlockedReason
  const selection = project.selection
  const [query, setQuery] = useState('')
  const [expanded, setExpanded] = useState<Set<string>>(() => new Set())
  const [renamingId, setRenamingId] = useState<string>()
  const contextMenu = useContextMenu()
  const normalizedQuery = query.trim().toLocaleLowerCase()
  const hierarchy = useMemo(() => normalizedQuery
    ? project.hierarchy.flatMap((node) => { const filtered = filterHierarchy(node, normalizedQuery); return filtered ? [filtered] : [] })
    : project.hierarchy,
  [normalizedQuery, project.hierarchy])
  const visible = useMemo(
    () => flattenHierarchy(hierarchy, expanded, Boolean(normalizedQuery)),
    [expanded, hierarchy, normalizedQuery],
  )
  const visibleIndex = useMemo(() => new Map(visible.map((item, index) => [item.node.id, index])), [visible])

  const select = (entityIds: string[], activeEntityId?: string) => {
    void controller.invoke('scene.select', { entityIds, entityId: activeEntityId })
  }
  const selectNode = (node: HierarchyNode, event: MouseEvent) => {
    if (event.shiftKey && selection.activeEntityId) {
      const anchor = visibleIndex.get(selection.activeEntityId)
      const target = visibleIndex.get(node.id)
      if (anchor !== undefined && target !== undefined) {
        const [start, end] = anchor < target ? [anchor, target] : [target, anchor]
        select(visible.slice(start, end + 1).map((item) => item.node.id), node.id)
        return
      }
    }
    if (event.ctrlKey || event.metaKey) {
      const next = selection.entityIds.includes(node.id)
        ? selection.entityIds.filter((id) => id !== node.id)
        : [...selection.entityIds, node.id]
      select(next, next.includes(node.id) ? node.id : next.at(-1))
      return
    }
    select([node.id], node.id)
  }
  const createEntity = (templateId = 'empty', parentId?: string) => {
    if (editing) void controller.invoke('scene.createEntity', { templateId, parentId })
  }
  const selectionFor = (node: HierarchyNode) => selection.entityIds.includes(node.id) ? selection.entityIds : [node.id]
  const entityMenu = (node: HierarchyNode): ContextMenuEntry[] => {
    const ids = selectionFor(node)
    const single = ids.length === 1
    const prefabAsset = node.prefab ? project.assets.find((asset) => asset.id === node.prefab) : undefined
    return [
      { id: 'create-child', label: 'Create Empty Child', disabled: !editing, onSelect: () => createEntity('empty', node.id) },
      { id: 'create-child-menu', label: 'Create Child', disabled: !editing, children: groupedTemplates(project.catalog.entityTemplates, (templateId) => createEntity(templateId, node.id)) },
      { type: 'separator', id: 'edit-start' },
      { id: 'cut', label: 'Cut', shortcut: 'Ctrl+X', disabled: !editing, onSelect: () => void controller.invoke('scene.cutEntities', { entityIds: ids }) },
      { id: 'copy', label: 'Copy', shortcut: 'Ctrl+C', onSelect: () => void controller.invoke('scene.copyEntities', { entityIds: ids }) },
      { id: 'paste-child', label: 'Paste as Child', shortcut: 'Ctrl+V', disabled: !editing || project.clipboard.entityRootCount === 0, disabledReason: project.clipboard.entityRootCount ? 'Stop Play mode before editing' : 'Entity clipboard is empty', onSelect: () => void controller.invoke('scene.pasteEntities', { parentId: node.id }) },
      { id: 'duplicate', label: 'Duplicate', shortcut: 'Ctrl+D', disabled: !editing, onSelect: () => void controller.invoke('scene.duplicateEntity', { entityIds: ids }) },
      { id: 'rename', label: 'Rename', shortcut: 'F2', disabled: !editing || !single, disabledReason: single ? 'Stop Play mode before editing' : 'Rename requires one GameObject', onSelect: () => setRenamingId(node.id) },
      { id: 'delete', label: ids.length > 1 ? `Delete ${ids.length} GameObjects` : 'Delete', shortcut: 'Del', danger: true, disabled: !editing, onSelect: () => void controller.invoke('scene.deleteEntity', { entityIds: ids }) },
      { type: 'separator', id: 'arrange-start' },
      { id: 'active', label: 'Active', checked: node.enabled, disabled: !editing, onSelect: () => void controller.invoke('scene.setEntityEnabled', { entityIds: ids, enabled: !node.enabled }) },
      { id: 'move', label: 'Move', disabled: !editing || !single, children: [
        { id: 'move-first', label: 'Move to First Sibling', onSelect: () => void controller.invoke('scene.moveEntity', { entityId: node.id, movement: 'first' }) },
        { id: 'move-up', label: 'Move Up', onSelect: () => void controller.invoke('scene.moveEntity', { entityId: node.id, movement: 'up' }) },
        { id: 'move-down', label: 'Move Down', onSelect: () => void controller.invoke('scene.moveEntity', { entityId: node.id, movement: 'down' }) },
        { id: 'move-last', label: 'Move to Last Sibling', onSelect: () => void controller.invoke('scene.moveEntity', { entityId: node.id, movement: 'last' }) },
      ] },
      { id: 'unparent', label: 'Move to Scene Root', disabled: !editing, onSelect: () => void controller.invoke('scene.setEntityParent', { entityIds: ids }) },
      { id: 'focus', label: 'Frame Selected', shortcut: 'F', onSelect: () => void controller.invoke('viewport.focusSelection', {}) },
      ...(node.prefab ? [
        { type: 'separator' as const, id: 'prefab-start' },
        { id: 'select-prefab', label: 'Select Prefab Asset', disabled: !prefabAsset, disabledReason: 'Prefab source is not in the current asset catalog', onSelect: () => prefabAsset && controller.selectAsset(prefabAsset.assetId) },
        { id: 'unpack-prefab', label: 'Unpack Prefab', disabled: !editing, onSelect: () => void controller.invoke('assets.unpackPrefab', { entityId: node.id, mode: 'instance' }) },
        { id: 'unpack-prefab-complete', label: 'Unpack Prefab Completely', disabled: !editing, onSelect: () => void controller.invoke('assets.unpackPrefab', { entityId: node.id, mode: 'completely' }) },
      ] : []),
    ]
  }
  const blankMenu = (): ContextMenuEntry[] => [
    { id: 'create-empty', label: 'Create Empty', disabled: !editing, onSelect: () => createEntity() },
    { id: 'create-menu', label: 'Create', disabled: !editing, children: groupedTemplates(project.catalog.entityTemplates, (templateId) => createEntity(templateId)) },
    { type: 'separator', id: 'blank-edit' },
    { id: 'paste-root', label: 'Paste', shortcut: 'Ctrl+V', disabled: !editing || project.clipboard.entityRootCount === 0, disabledReason: project.clipboard.entityRootCount ? 'Stop Play mode before editing' : 'Entity clipboard is empty', onSelect: () => void controller.invoke('scene.pasteEntities', {}) },
    { id: 'select-all', label: 'Select All', shortcut: 'Ctrl+A', disabled: visible.length === 0, onSelect: () => select(visible.map((item) => item.node.id), visible[0]?.node.id) },
  ]

  const keyboard = (node: HierarchyNode, event: KeyboardEvent<HTMLDivElement>) => {
    const index = visibleIndex.get(node.id) ?? -1
    const control = event.ctrlKey || event.metaKey
    if (event.key === 'F2' && editing) { event.preventDefault(); setRenamingId(node.id); return }
    if (event.key === 'Delete' && editing) { event.preventDefault(); void controller.invoke('scene.deleteEntity', { entityIds: selectionFor(node) }); return }
    if (control && event.key.toLocaleLowerCase() === 'd' && editing) { event.preventDefault(); void controller.invoke('scene.duplicateEntity', { entityIds: selectionFor(node) }); return }
    if (control && event.key.toLocaleLowerCase() === 'c') { event.preventDefault(); void controller.invoke('scene.copyEntities', { entityIds: selectionFor(node) }); return }
    if (control && event.key.toLocaleLowerCase() === 'x' && editing) { event.preventDefault(); void controller.invoke('scene.cutEntities', { entityIds: selectionFor(node) }); return }
    if (control && event.key.toLocaleLowerCase() === 'v' && editing && project.clipboard.entityRootCount) { event.preventDefault(); void controller.invoke('scene.pasteEntities', { parentId: node.id }); return }
    if (event.key === 'ContextMenu' || (event.shiftKey && event.key === 'F10')) {
      event.preventDefault()
      const bounds = event.currentTarget.getBoundingClientRect()
      contextMenu.openContextMenu({ clientX: bounds.left + 24, clientY: bounds.bottom, currentTarget: event.currentTarget, preventDefault() {}, stopPropagation() {} }, entityMenu(node), { ariaLabel: `${node.name} actions` })
      return
    }
    const adjacent = event.key === 'ArrowUp' ? visible[index - 1] : event.key === 'ArrowDown' ? visible[index + 1] : undefined
    if (adjacent) { event.preventDefault(); select([adjacent.node.id], adjacent.node.id); return }
    if (event.key === 'ArrowRight' && node.children.length) { event.preventDefault(); if (!expanded.has(node.id)) setExpanded((current) => new Set(current).add(node.id)); else select([node.children[0]!.id], node.children[0]!.id) }
    if (event.key === 'ArrowLeft') { event.preventDefault(); if (expanded.has(node.id)) setExpanded((current) => { const next = new Set(current); next.delete(node.id); return next }) }
  }

  return <div className="panel-column hierarchy-panel" data-editor-context="hierarchy">
    <div className="panel-action-row">
      <div className="compact-search"><Icon name="search" /><input value={query} placeholder="Search" onChange={(event) => setQuery(event.target.value)} />{query && <button type="button" onClick={() => setQuery('')}><Icon name="close" /></button>}</div>
      <button className="square-action" type="button" title={editing ? 'Create GameObject' : authoringBlockedReason} disabled={!editing} onClick={() => createEntity()}><Icon name="add" /></button>
    </div>
    <div
      className="hierarchy-tree panel-scroll"
      role="tree"
      aria-label="Scene hierarchy"
      onClick={(event) => { if (event.target === event.currentTarget) select([]) }}
      onContextMenu={(event) => contextMenu.openContextMenu(event, blankMenu(), { ariaLabel: 'Hierarchy actions' })}
      onDragOver={(event) => { if (editing) event.preventDefault() }}
      onDrop={(event) => {
        event.preventDefault()
        if (!editing) return
        const entityIds = parseDraggedEntities(event.dataTransfer.getData('application/x-engine-entities'))
        if (entityIds.length) void controller.invoke('scene.setEntityParent', { entityIds })
      }}
    >
      {visible.map((item) => <HierarchyRow key={item.node.id} item={item} selected={selection.entityIds.includes(item.node.id)} expanded={Boolean(normalizedQuery) || expanded.has(item.node.id)} editing={editing} renaming={renamingId === item.node.id} selectedIds={selection.entityIds} controller={controller} onSelect={selectNode} onKeyboard={keyboard} onToggle={(id) => setExpanded((current) => { const next = new Set(current); if (next.has(id)) next.delete(id); else next.add(id); return next })} onRename={setRenamingId} onMenu={(event, node) => {
        if (!selection.entityIds.includes(node.id)) select([node.id], node.id)
        contextMenu.openContextMenu(event, entityMenu(node), { ariaLabel: `${node.name} actions` })
      }} />)}
      {visible.length === 0 && <div className="panel-empty"><Icon name="hierarchy" /><span>{normalizedQuery ? 'No matching GameObjects' : 'The scene has no GameObjects'}</span>{!normalizedQuery && <button type="button" disabled={!editing} onClick={() => createEntity()}>Create GameObject</button>}</div>}
    </div>
    <div className="hierarchy-footer"><span>{project.hierarchy.length} root objects · {selection.entityIds.length} selected</span></div>
    <ContextMenu request={contextMenu.request} onClose={contextMenu.closeContextMenu} />
  </div>
}
