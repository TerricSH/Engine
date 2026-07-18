import { useEffect, useMemo, useRef, useState, type KeyboardEvent as ReactKeyboardEvent } from 'react'
import type { AssetKind, AssetSnapshot } from '../bridge/protocol'
import { engineBridge } from '../bridge/engineBridge'
import type { EditorController } from '../state/useEditorState'
import { ContextMenu, useContextMenu, type ContextMenuEntry, type ContextMenuTriggerEvent } from '../components/ContextMenu'
import { openWorkflowDialog, sceneForAsset, type AssetWorkflowTarget } from '../components/WorkflowDialogs'
import { Icon, type IconName } from '../components/Icon'

const assetIcons: Record<AssetKind, IconName> = {
  scene: 'scene', prefab: 'asset', model: 'cube', material: 'sphere', texture: 'scene',
  audio: 'asset', navmesh: 'asset', script: 'console', shader: 'asset', other: 'asset',
}

function sourceFolder(folder: string): string {
  return folder.replace(/^\/+/, '')
}

function isTextEntry(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement
    || target instanceof HTMLTextAreaElement
    || target instanceof HTMLSelectElement
    || (target instanceof HTMLElement && target.isContentEditable)
}

export function ProjectPanel({ controller }: { controller: EditorController }) {
  const project = controller.state.project
  const browser = project.assetBrowser
  const contextMenu = useContextMenu()
  const [query, setQuery] = useState(browser.query)
  const [operationBusy, setOperationBusy] = useState(false)
  const [operationMessage, setOperationMessage] = useState<string>()
  const [operationError, setOperationError] = useState<string>()
  const sessionRef = useRef(project.sessionId)
  const selectedAsset = project.assets.find((asset) => asset.id === browser.selectedAsset)
  const assetSourceFolder = sourceFolder(browser.folder)
  const writeBlockedReason = !project.capabilities.editing
    ? 'Stop Play mode before changing project content.'
    : project.capabilities.buildBusy
      ? 'Wait for the active project operation to finish.'
      : operationBusy
        ? 'Wait for the current Project operation to finish.'
        : undefined

  useEffect(() => {
    if (sessionRef.current === project.sessionId) return
    sessionRef.current = project.sessionId
    setQuery(browser.query)
    contextMenu.closeContextMenu()
  }, [browser.query, contextMenu.closeContextMenu, project.sessionId])

  useEffect(() => {
    if (!project.sessionId || query === browser.query) return
    const timeout = window.setTimeout(() => { void controller.invoke('assets.setBrowser', { query, page: 0 }) }, 250)
    return () => window.clearTimeout(timeout)
  }, [browser.query, controller.invoke, project.sessionId, query])

  const visibleAssets = useMemo(() => {
    const byId = new Map(project.assets.map((asset) => [asset.id, asset]))
    return browser.visibleAssetIds.flatMap((id) => {
      const asset = byId.get(id)
      return asset ? [asset] : []
    })
  }, [browser.visibleAssetIds, project.assets])

  const runOperation = async (label: string, operation: () => Promise<unknown | undefined>) => {
    setOperationBusy(true)
    setOperationMessage(undefined)
    setOperationError(undefined)
    try {
      const result = await operation()
      if (result === undefined) {
        setOperationError(`${label} failed. Review the editor error banner or Console for details.`)
        return false
      }
      if (typeof result === 'object'
        && result !== null
        && 'accepted' in result
        && result.accepted === false) {
        setOperationError(`${label} was rejected. Review the editor error banner or Console for details.`)
        return false
      }
      const jobId = typeof result === 'object'
        && result !== null
        && 'jobId' in result
        && typeof result.jobId === 'number'
        ? result.jobId
        : undefined
      if (jobId !== undefined) {
        const deadline = Date.now() + 5 * 60_000
        while (Date.now() < deadline) {
          await new Promise((resolve) => window.setTimeout(resolve, 80))
          const snapshot = await engineBridge.invoke('editor.getSnapshot', {})
          const status = snapshot.backgroundOperations?.find((operation) => operation.id === jobId)
            ?? (snapshot.backgroundOperation?.id === jobId ? snapshot.backgroundOperation : undefined)
          if (!status || status.state === 'running') continue
          if (status.state === 'failed') {
            setOperationError(status.error ?? `${status.label} failed.`)
            return false
          }
          if (status.state === 'committedWithWarning') {
            setOperationMessage(status.error ?? `${status.label} committed, but the editor refresh needs attention.`)
            return true
          }
          setOperationMessage(`${status.label} completed.`)
          return true
        }
        setOperationError(`${label} did not report completion within five minutes.`)
        return false
      }
      setOperationMessage(`${label} completed.`)
      return true
    } catch (error) {
      setOperationError(`${label} failed: ${error instanceof Error ? error.message : String(error)}`)
      return false
    } finally {
      setOperationBusy(false)
    }
  }

  const copyText = async (label: string, value: string) => {
    setOperationMessage(undefined)
    setOperationError(undefined)
    try {
      if (!navigator.clipboard?.writeText) throw new Error('Clipboard access is unavailable in this editor host.')
      await navigator.clipboard.writeText(value)
      setOperationMessage(`${label} copied.`)
    } catch (error) {
      setOperationError(`Could not copy ${label.toLocaleLowerCase()}: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  const target = (asset: AssetSnapshot): AssetWorkflowTarget => ({
    id: asset.id,
    name: asset.name,
    path: asset.path,
    kind: asset.kind,
  })
  const catalogScene = (asset: AssetSnapshot) => sceneForAsset(project, asset)

  const reportWriteBlocked = () => {
    if (!writeBlockedReason) return false
    setOperationMessage(undefined)
    setOperationError(writeBlockedReason)
    return true
  }

  const openAsset = (asset: AssetSnapshot) => {
    const scene = catalogScene(asset)
    if (scene) {
      if (reportWriteBlocked()) return
      void runOperation('Open scene', () => controller.invoke('document.open', { sceneId: scene.id }))
      return
    }
    void runOperation('Open asset', () => controller.invoke('assets.open', { assetId: asset.id }))
  }

  const duplicateAsset = (asset: AssetSnapshot) => {
    if (reportWriteBlocked()) return
    const scene = catalogScene(asset)
    if (scene) {
      openWorkflowDialog({ kind: 'duplicateScene', sceneId: scene.id })
      return
    }
    if (!asset.manifestDeclared) {
      setOperationError('Only assets declared in a source manifest can be duplicated.')
      return
    }
    void runOperation('Duplicate asset', () => controller.invoke('assets.duplicate', { assetId: asset.id }))
  }

  const moveAsset = (asset: AssetSnapshot) => {
    if (reportWriteBlocked()) return
    const scene = catalogScene(asset)
    if (scene) {
      openWorkflowDialog({ kind: 'renameScene', sceneId: scene.id })
      return
    }
    if (!asset.manifestDeclared) {
      setOperationError('Only assets declared in a source manifest can be moved or renamed.')
      return
    }
    openWorkflowDialog({ kind: 'moveAsset', asset: target(asset) })
  }

  const deleteAsset = (asset: AssetSnapshot) => {
    if (reportWriteBlocked()) return
    const scene = catalogScene(asset)
    if (scene) {
      openWorkflowDialog({ kind: 'deleteScene', sceneId: scene.id })
      return
    }
    if (!asset.manifestDeclared) {
      setOperationError('Only assets declared in a source manifest can be deleted.')
      return
    }
    openWorkflowDialog({ kind: 'deleteAsset', asset: target(asset) })
  }

  const createEntries = (folder: string): ContextMenuEntry[] => {
    const relativeFolder = sourceFolder(folder)
    return [
      {
        id: 'create-folder',
        label: 'Folder…',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason,
        onSelect: () => openWorkflowDialog({ kind: 'createFolder', folder: relativeFolder }),
      },
      {
        id: 'create-material',
        label: 'Material…',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason,
        onSelect: () => openWorkflowDialog({ kind: 'createMaterial', folder: relativeFolder }),
      },
      {
        id: 'create-script',
        label: 'C# Script…',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason,
        onSelect: () => openWorkflowDialog({ kind: 'createScript' }),
      },
      {
        id: 'create-prefab',
        label: 'Prefab from Selection…',
        disabled: Boolean(writeBlockedReason) || !project.capabilities.hasSelection,
        disabledReason: writeBlockedReason ?? (!project.capabilities.hasSelection ? 'Select an entity hierarchy first.' : undefined),
        onSelect: () => openWorkflowDialog({ kind: 'createPrefab', folder: relativeFolder }),
      },
    ]
  }

  const folderMenuEntries = (folder: string, includeProjectRootActions: boolean): ContextMenuEntry[] => {
    const relativeFolder = sourceFolder(folder)
    const label = relativeFolder ? `Folder · ${relativeFolder}` : 'Assets Root'
    const entries: ContextMenuEntry[] = [
      { id: 'folder-context', label, disabled: true, disabledReason: 'Current Project folder' },
      { type: 'separator', id: 'folder-header-separator' },
      { id: 'create-here', label: 'Create', children: createEntries(relativeFolder) },
      {
        id: 'import-here',
        label: 'Import New Asset…',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason,
        onSelect: () => openWorkflowDialog({ kind: 'importAsset', folder: relativeFolder }),
      },
      { type: 'separator', id: 'folder-refresh-separator' },
      {
        id: 'refresh-all',
        label: 'Refresh All',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason,
        onSelect: () => void runOperation('Refresh assets', () => controller.invoke('assets.refresh', {})),
      },
    ]
    if (!includeProjectRootActions) {
      entries.push(
        { type: 'separator', id: 'folder-mutation-separator' },
        {
          id: 'rename-folder',
          label: 'Rename',
          shortcut: 'F2',
          disabled: Boolean(writeBlockedReason),
          disabledReason: writeBlockedReason,
          onSelect: () => openWorkflowDialog({ kind: 'renameFolder', folder: relativeFolder }),
        },
        {
          id: 'delete-folder',
          label: 'Delete',
          shortcut: 'Del',
          danger: true,
          disabled: Boolean(writeBlockedReason),
          disabledReason: writeBlockedReason,
          onSelect: () => openWorkflowDialog({ kind: 'deleteFolder', folder: relativeFolder }),
        },
        { type: 'separator', id: 'folder-reveal-separator' },
        {
          id: 'show-folder',
          label: 'Show in Explorer',
          disabled: operationBusy,
          disabledReason: operationBusy ? 'Wait for the current Project operation to finish.' : undefined,
          onSelect: () => void runOperation('Reveal asset folder', () => controller.invoke('assets.revealFolder', { folder: relativeFolder })),
        },
      )
    }
    if (includeProjectRootActions) {
      entries.push(
        { type: 'separator', id: 'project-root-separator' },
        {
          id: 'show-project-root',
          label: 'Show Project Root in Explorer',
          disabled: operationBusy,
          disabledReason: operationBusy ? 'Wait for the current Project operation to finish.' : undefined,
          onSelect: () => void runOperation('Reveal project root', () => controller.invoke('project.reveal', {})),
        },
        {
          id: 'copy-project-path',
          label: 'Copy Project Path',
          onSelect: () => copyText('Project path', project.projectPath),
        },
      )
    }
    return entries
  }

  const assetMenuEntries = (asset: AssetSnapshot): ContextMenuEntry[] => {
    const scene = catalogScene(asset)
    const classification = scene
      ? 'Scene Document'
      : asset.kind === 'scene'
        ? 'Source Scene Asset'
        : `${asset.kind.charAt(0).toLocaleUpperCase()}${asset.kind.slice(1)} Asset`
    const sourceMutationReason = writeBlockedReason
      ?? (!asset.manifestDeclared ? 'This asset is not declared in a source manifest.' : undefined)
    const openReason = scene ? writeBlockedReason : operationBusy ? 'Wait for the current Project operation to finish.' : undefined
    const copyPath = scene?.path ?? (asset.manifestDeclared ? `assets/source/${asset.path.replace(/^\/+/, '')}` : asset.path)
    const entries: ContextMenuEntry[] = [
      {
        id: 'asset-context',
        label: `${asset.name} · ${classification}`,
        disabled: true,
        disabledReason: asset.path,
      },
      { type: 'separator', id: 'asset-header-separator' },
      {
        id: 'open',
        label: scene ? 'Open Scene' : 'Open',
        disabled: Boolean(openReason) || Boolean(scene?.current),
        disabledReason: scene?.current ? 'This scene is already open.' : openReason,
        onSelect: () => openAsset(asset),
      },
    ]

    if (scene) {
      entries.push({
        id: 'set-startup',
        label: 'Set as Startup Scene',
        checked: scene.startup,
        disabled: Boolean(writeBlockedReason) || scene.startup,
        disabledReason: scene.startup ? 'This is already the startup scene.' : writeBlockedReason,
        onSelect: () => void runOperation('Set startup scene', () => controller.invoke('document.setStartup', { sceneId: scene.id })),
      })
    }
    if (asset.kind === 'material') {
      const assignReason = writeBlockedReason
        ?? (!project.selection.activeEntityId ? 'Select an entity with a Mesh Renderer first.' : undefined)
        ?? (!asset.loaded ? 'This material is not loaded. Refresh project assets first.' : undefined)
      entries.push({
        id: 'assign-material',
        label: 'Assign to Selected GameObject',
        disabled: Boolean(assignReason),
        disabledReason: assignReason,
        onSelect: () => {
          const entityId = project.selection.activeEntityId
          if (entityId) void runOperation('Assign material', () => controller.invoke('assets.assign', { assetId: asset.id, entityId }))
        },
      })
    }
    if (asset.kind === 'prefab') {
      const instantiateReason = writeBlockedReason
        ?? (!asset.loaded ? 'This prefab is not loaded. Refresh project assets first.' : undefined)
      entries.push({
        id: 'instantiate-prefab',
        label: 'Instantiate Prefab',
        disabled: Boolean(instantiateReason),
        disabledReason: instantiateReason,
        onSelect: () => void runOperation('Instantiate prefab', () => controller.invoke('assets.instantiatePrefab', { assetId: asset.assetId })),
      })
    }

    entries.push(
      { type: 'separator', id: 'asset-mutation-separator' },
      {
        id: 'duplicate',
        label: scene ? 'Duplicate Scene…' : 'Duplicate',
        shortcut: 'Ctrl+D',
        disabled: Boolean(scene ? writeBlockedReason : sourceMutationReason),
        disabledReason: scene ? writeBlockedReason : sourceMutationReason,
        onSelect: () => duplicateAsset(asset),
      },
      {
        id: 'rename',
        label: scene ? 'Rename Scene…' : 'Move / Rename…',
        shortcut: 'F2',
        disabled: Boolean(scene ? writeBlockedReason : sourceMutationReason),
        disabledReason: scene ? writeBlockedReason : sourceMutationReason,
        onSelect: () => moveAsset(asset),
      },
      {
        id: 'delete',
        label: scene ? 'Delete Scene…' : 'Delete Asset…',
        shortcut: 'Delete',
        danger: true,
        disabled: Boolean(scene ? writeBlockedReason : sourceMutationReason),
        disabledReason: scene ? writeBlockedReason : sourceMutationReason,
        onSelect: () => deleteAsset(asset),
      },
      { type: 'separator', id: 'asset-file-separator' },
      {
        id: 'refresh-all',
        label: 'Refresh All',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason ?? 'Per-asset reimport is not exposed by the native bridge.',
        onSelect: () => void runOperation('Refresh assets', () => controller.invoke('assets.refresh', {})),
      },
      {
        id: 'reveal',
        label: 'Show in Explorer',
        disabled: operationBusy,
        disabledReason: operationBusy ? 'Wait for the current Project operation to finish.' : undefined,
        onSelect: () => void runOperation('Reveal asset', () => controller.invoke('assets.reveal', { assetId: asset.id })),
      },
      {
        id: 'copy-path',
        label: 'Copy Path',
        onSelect: () => copyText('Asset path', copyPath),
      },
    )
    if (asset.kind === 'script') {
      entries.push({
        id: 'rebuild-scripts',
        label: 'Rebuild Scripts',
        disabled: Boolean(writeBlockedReason),
        disabledReason: writeBlockedReason,
        onSelect: () => void runOperation('Rebuild scripts', () => controller.invoke('script.rebuild', {})),
      })
    }
    return entries
  }

  const showAssetMenu = (asset: AssetSnapshot, event: ContextMenuTriggerEvent) => {
    controller.selectAsset(asset.assetId)
    contextMenu.openContextMenu(event, assetMenuEntries(asset), {
      ariaLabel: `${asset.name} Project actions`,
    })
  }

  const showFolderMenu = (folder: string, event: ContextMenuTriggerEvent) => {
    controller.selectAsset()
    void controller.invoke('assets.setBrowser', { folder, page: 0 })
    contextMenu.openContextMenu(event, folderMenuEntries(folder, !sourceFolder(folder)), {
      ariaLabel: `${sourceFolder(folder) || 'Assets'} folder actions`,
    })
  }

  const showBlankMenu = (event: ContextMenuTriggerEvent) => {
    controller.selectAsset()
    contextMenu.openContextMenu(event, folderMenuEntries(browser.folder, true), {
      ariaLabel: 'Project folder actions',
    })
  }

  const selectFolder = (folder: string) => {
    controller.selectAsset()
    void controller.invoke('assets.setBrowser', { folder, page: 0 })
  }
  const setView = (view: 'grid' | 'list') => { void controller.invoke('assets.setBrowser', { view }) }

  const handleProjectKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    if (isTextEntry(event.target) || !selectedAsset) return
    const control = event.ctrlKey || event.metaKey
    const key = event.key.toLocaleLowerCase()
    if (event.key === 'Delete') {
      event.preventDefault()
      event.stopPropagation()
      deleteAsset(selectedAsset)
    } else if (event.key === 'F2') {
      event.preventDefault()
      event.stopPropagation()
      moveAsset(selectedAsset)
    } else if (control && key === 'd') {
      event.preventDefault()
      event.stopPropagation()
      duplicateAsset(selectedAsset)
    } else if (event.key === 'Enter') {
      event.preventDefault()
      event.stopPropagation()
      openAsset(selectedAsset)
    }
  }

  return <div className="project-panel panel-column" data-editor-context="project" onKeyDown={handleProjectKeyDown}>
    <div className="project-toolbar">
      <button type="button" title="Refresh and recook project assets" disabled={Boolean(writeBlockedReason)} onClick={() => void runOperation('Refresh assets', () => controller.invoke('assets.refresh', {}))}><Icon name="refresh" /></button>
      <div className="breadcrumb"><button type="button" onClick={() => selectFolder('/')}>Assets</button>{browser.folder.split('/').filter(Boolean).map((segment, index, segments) => <button key={`${segment}-${index}`} type="button" onClick={() => selectFolder(`/${segments.slice(0, index + 1).join('/')}`)}><Icon name="chevron" />{segment}</button>)}</div>
      <div className="project-toolbar-spacer" />
      <div className="compact-search project-search"><Icon name="search" /><input value={query} placeholder="Search assets" onChange={(event) => setQuery(event.target.value)} /></div>
      <select className="asset-kind-filter" value={browser.kindFilter.toLocaleLowerCase()} title="Asset type filter" onChange={(event) => void controller.invoke('assets.setBrowser', { kind: event.target.value, page: 0 })}>
        <option value="all">All types</option><option value="scene">Scenes</option><option value="prefab">Prefabs</option><option value="mesh">Models</option><option value="material">Materials</option><option value="texture">Textures</option><option value="script">Scripts</option><option value="audio">Audio</option><option value="shader">Shaders</option><option value="other">Other</option>
      </select>
      <button className={browser.view === 'grid' ? 'active' : ''} type="button" title="Grid" onClick={() => setView('grid')}>▦</button>
      <button className={browser.view === 'list' ? 'active' : ''} type="button" title="List" onClick={() => setView('list')}>☷</button>
      <button type="button" disabled={Boolean(writeBlockedReason)} onClick={() => openWorkflowDialog({ kind: 'importAsset', folder: assetSourceFolder })}>Import</button>
      <button type="button" disabled={Boolean(writeBlockedReason)} title="Create asset" onClick={(event) => contextMenu.openContextMenu(event, createEntries(assetSourceFolder), { ariaLabel: 'Create Project asset' })}><Icon name="add" /></button>
      <button type="button" disabled={!selectedAsset} title="Selected asset actions" onClick={(event) => selectedAsset && showAssetMenu(selectedAsset, event)}>⋮</button>
    </div>
    {(operationError || operationMessage) && <div className={operationError ? 'project-operation-feedback error' : 'project-operation-feedback'} role={operationError ? 'alert' : 'status'}>{operationError ?? operationMessage}<button type="button" aria-label="Dismiss" onClick={() => { setOperationError(undefined); setOperationMessage(undefined) }}>×</button></div>}
    <div className="project-content">
      <div className="folder-tree panel-scroll">
        <button type="button" className={browser.folder === '/' || !browser.folder ? 'active' : ''} onClick={() => selectFolder('/')} onContextMenu={(event) => showFolderMenu('/', event)}><Icon name="folder" /><span>Assets</span></button>
        {browser.folders.filter((folder) => folder.path !== '/').map((folder) => <button type="button" key={folder.path} className={folder.path === browser.folder ? 'active' : ''} style={{ paddingInlineStart: `${10 + folder.depth * 15}px` }} onClick={() => selectFolder(folder.path)} onContextMenu={(event) => showFolderMenu(folder.path, event)}><Icon className="folder-chevron" name="chevron" /><Icon name="folder" /><span>{folder.name}</span><small>{folder.directAssetCount}</small></button>)}
      </div>
      <div
        className={`asset-browser panel-scroll ${browser.view}`}
        onPointerDown={(event) => {
          if (!(event.target instanceof Element) || !event.target.closest('.asset-item')) controller.selectAsset()
        }}
        onContextMenu={(event) => showBlankMenu(event)}
      >
        {visibleAssets.map((asset) => <button type="button" key={asset.id} className={browser.selectedAsset === asset.id ? 'asset-item selected' : 'asset-item'} draggable title={asset.path} onClick={() => controller.selectAsset(asset.assetId)} onDoubleClick={() => openAsset(asset)} onContextMenu={(event) => showAssetMenu(asset, event)} onKeyDown={(event) => { if (event.key === 'Enter') { event.stopPropagation(); openAsset(asset) } }} onDragStart={(event) => { event.dataTransfer.setData('application/x-engine-assets', JSON.stringify(asset.assetId)); event.dataTransfer.effectAllowed = 'copyLink' }}>
          <span className={`asset-thumbnail kind-${asset.kind}`}><Icon name={assetIcons[asset.kind]} />{(!asset.cooked || !asset.loaded) && <span className="modified-dot" title={!asset.cooked ? 'Not cooked' : 'Not loaded'} />}</span>
          <span className="asset-name">{asset.name}</span>{browser.view === 'list' && <><span className="asset-kind">{asset.kind}</span><span className="asset-path">{asset.path}</span></>}
        </button>)}
        {visibleAssets.length === 0 && <div className="panel-empty"><Icon name="folder" /><span>{query ? 'No assets match this search' : 'This folder is empty'}</span></div>}
      </div>
    </div>
    <div className="project-footer">
      <span>{browser.total === 0 ? '0' : `${browser.page * browser.pageSize + 1}–${browser.page * browser.pageSize + visibleAssets.length}`} / {browser.total} items</span>
      <button type="button" disabled={browser.page === 0} onClick={() => void controller.invoke('assets.setBrowser', { page: browser.page - 1 })}>Previous</button>
      <span>Page {browser.pageCount === 0 ? 0 : browser.page + 1} / {browser.pageCount}</span>
      <button type="button" disabled={browser.page + 1 >= browser.pageCount} onClick={() => void controller.invoke('assets.setBrowser', { page: browser.page + 1 })}>Next</button>
      <span>{browser.selectedAsset ? '1 selected' : '0 selected'}</span>{(operationBusy || project.capabilities.buildBusy) && <span>Working…</span>}
    </div>
    <ContextMenu request={contextMenu.request} onClose={contextMenu.closeContextMenu} />
  </div>
}
