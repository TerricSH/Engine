import { useCallback, useEffect, useMemo, useState } from 'react'
import { CommandPalette } from './components/CommandPalette'
import { DockWorkspace } from './components/DockWorkspace'
import { MainToolbar } from './components/MainToolbar'
import { MenuBar } from './components/MenuBar'
import { StatusBar } from './components/StatusBar'
import { WorkflowDialogs, WORKFLOW_DIALOG_EVENT, type WorkflowRequest } from './components/WorkflowDialogs'
import { engineBridge } from './bridge/engineBridge'
import type { RuntimeMode, TransformTool } from './bridge/protocol'
import type { DockZoneId, PanelId } from './layout/dockLayout'
import { useDockLayout } from './layout/dockLayout'
import { editorShortcutAllowedForViewport, type NativeViewportKind } from './keyboardRouting'
import { useEditorState } from './state/useEditorState'

const windowPanels: Partial<Record<string, { panel: PanelId; zone: DockZoneId }>> = {
  'window.scene': { panel: 'scene', zone: 'center' }, 'window.game': { panel: 'game', zone: 'center' },
  'window.hierarchy': { panel: 'hierarchy', zone: 'left' }, 'window.inspector': { panel: 'inspector', zone: 'right' },
  'window.project': { panel: 'project', zone: 'bottom' }, 'window.console': { panel: 'console', zone: 'bottom' },
  'window.material': { panel: 'material', zone: 'bottom' },
  'window.animation': { panel: 'animation', zone: 'bottom' }, 'window.profiler': { panel: 'profiler', zone: 'bottom' },
  'window.build': { panel: 'build', zone: 'bottom' },
}

function isTextEntry(target: EventTarget | null): boolean {
  return target instanceof HTMLInputElement || target instanceof HTMLTextAreaElement || target instanceof HTMLSelectElement || (target instanceof HTMLElement && target.isContentEditable)
}

export function App() {
  const controller = useEditorState()
  const { project } = controller.state
  const dock = useDockLayout(project.sessionId, project.workspace.reactLayout)
  const [paletteOpen, setPaletteOpen] = useState(false)
  const [workflow, setWorkflow] = useState<WorkflowRequest>()
  const entityIds = project.selection.entityIds
  const activeEntityId = project.selection.activeEntityId
  const assetSourceFolder = project.assetBrowser.folder.replace(/^\/+/, '')
  const disabledCommands = useMemo(() => {
    const disabled = new Set<string>()
    if (!project.capabilities.editing) {
      ;[
        'file.newScene', 'file.openScene', 'file.saveScene', 'file.saveSceneAs',
        'edit.undo', 'edit.redo', 'edit.cut', 'edit.copy', 'edit.paste', 'edit.duplicate', 'edit.delete',
        'assets.refresh', 'assets.import', 'assets.createFolder', 'assets.createMaterial', 'assets.createScript', 'assets.createPrefab',
        'gameObject.empty', 'gameObject.cube', 'gameObject.camera', 'gameObject.light', 'gameObject.audioListener',
        'component.add', 'component.resetTransform',
      ].forEach((command) => disabled.add(command))
    }
    if (project.capabilities.buildBusy) {
      ;[
        'file.newScene', 'file.openScene', 'file.saveScene', 'file.saveSceneAs',
        'edit.undo', 'edit.redo', 'edit.cut', 'edit.paste', 'edit.duplicate', 'edit.delete',
        'assets.refresh', 'assets.import', 'assets.createFolder', 'assets.createMaterial', 'assets.createScript', 'assets.createPrefab',
        'gameObject.empty', 'gameObject.cube', 'gameObject.camera', 'gameObject.light', 'gameObject.audioListener',
        'component.add', 'component.resetTransform',
      ].forEach((command) => disabled.add(command))
    }
    if (!project.capabilities.canSave) disabled.add('file.saveScene')
    if (!project.capabilities.canUndo) disabled.add('edit.undo')
    if (!project.capabilities.canRedo) disabled.add('edit.redo')
    if (entityIds.length === 0) { disabled.add('edit.cut'); disabled.add('edit.copy') }
    if (!activeEntityId) {
      ;['edit.duplicate', 'edit.delete', 'component.add', 'component.resetTransform', 'viewport.focusSelection'].forEach((command) => disabled.add(command))
    }
    if (!project.capabilities.hasSelection) disabled.add('assets.createPrefab')
    if (!project.assetBrowser.selectedAsset) disabled.add('assets.reveal')
    return disabled
  }, [activeEntityId, entityIds.length, project.assetBrowser.selectedAsset, project.capabilities])

  const setRuntimeMode = useCallback((mode: RuntimeMode) => { void controller.invoke('runtime.setMode', { mode }) }, [controller])
  const executeCommand = useCallback((command: string) => {
    if (disabledCommands.has(command)) return
    const panel = windowPanels[command]
    if (panel) { dock.show(panel.panel, panel.zone); return }
    switch (command) {
      case 'file.newScene': setWorkflow({ kind: 'newScene' }); break
      case 'file.openScene': setWorkflow({ kind: 'sceneManager' }); break
      case 'file.createProject': setWorkflow({ kind: 'createProject' }); break
      case 'file.openProject': setWorkflow({ kind: 'openProject' }); break
      case 'file.saveScene': void controller.invoke('document.save', {}); break
      case 'file.saveSceneAs': setWorkflow({ kind: 'saveSceneAs' }); break
      case 'file.build': dock.show('build', 'bottom'); break
      case 'file.quit': void controller.invoke('editor.quit', {}); break
      case 'edit.undo': void controller.invoke('scene.undo', {}); break
      case 'edit.redo': void controller.invoke('scene.redo', {}); break
      case 'edit.cut': if (entityIds.length) void controller.invoke('scene.cutEntities', { entityIds }); break
      case 'edit.copy': if (entityIds.length) void controller.invoke('scene.copyEntities', { entityIds }); break
      case 'edit.paste': void controller.invoke('scene.pasteEntities', {}); break
      case 'edit.duplicate': if (entityIds.length) void controller.invoke('scene.duplicateEntity', { entityIds }); break
      case 'edit.delete': if (entityIds.length) void controller.invoke('scene.deleteEntity', { entityIds }); break
      case 'edit.projectSettings': dock.show('settings', 'right'); break
      case 'assets.refresh': void controller.invoke('assets.refresh', {}); break
      case 'assets.import': setWorkflow({ kind: 'importAsset', folder: assetSourceFolder }); break
      case 'assets.createFolder': setWorkflow({ kind: 'createFolder', folder: assetSourceFolder }); break
      case 'assets.createMaterial': setWorkflow({ kind: 'createMaterial', folder: assetSourceFolder }); break
      case 'assets.createScript': setWorkflow({ kind: 'createScript' }); break
      case 'assets.createPrefab': setWorkflow({ kind: 'createPrefab', folder: assetSourceFolder }); break
      case 'assets.reveal': if (project.assetBrowser.selectedAsset) void controller.invoke('assets.reveal', { assetId: project.assetBrowser.selectedAsset }); break
      case 'gameObject.empty': void controller.invoke('scene.createEntity', {}); break
      case 'gameObject.cube': void controller.invoke('scene.createEntity', { templateId: 'cube' }); break
      case 'gameObject.camera': void controller.invoke('scene.createEntity', { templateId: 'camera' }); break
      case 'gameObject.light': void controller.invoke('scene.createEntity', { templateId: 'directional-light' }); break
      case 'gameObject.audioListener': void controller.invoke('scene.createEntity', { templateId: 'audio-listener' }); break
      case 'component.add': dock.show('inspector', 'right'); window.dispatchEvent(new CustomEvent('editor-open-add-component')); break
      case 'component.resetTransform': if (entityIds.length) void controller.invoke('scene.resetComponent', { entityIds, componentType: 'engine.transform' }); break
      case 'window.resetLayout': dock.reset(); break
      case 'viewport.focusSelection': void controller.invoke('viewport.focusSelection', {}); break
    }
  }, [activeEntityId, assetSourceFolder, controller, disabledCommands, dock, entityIds, project.assetBrowser.selectedAsset])

  useEffect(() => {
    const openWorkflow = (event: Event) => setWorkflow((event as CustomEvent<WorkflowRequest>).detail)
    window.addEventListener(WORKFLOW_DIALOG_EVENT, openWorkflow)
    return () => window.removeEventListener(WORKFLOW_DIALOG_EVENT, openWorkflow)
  }, [])

  useEffect(() => engineBridge.subscribeUiOpenPanel(({ panel, preferredZone }) => {
    dock.show(panel, preferredZone)
  }), [dock.show])

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      const control = event.ctrlKey || event.metaKey
      if (control && event.key.toLocaleLowerCase() === 'k') { event.preventDefault(); setPaletteOpen(true); return }
      if (event.key === 'Escape' && paletteOpen) { setPaletteOpen(false); return }
      if (isTextEntry(event.target)) return
      const nativeViewport = event.target instanceof Element
        ? event.target.closest<HTMLElement>('[data-native-viewport]')?.dataset.nativeViewport as NativeViewportKind | undefined
        : undefined
      if (!editorShortcutAllowedForViewport(nativeViewport, event.key)) return
      const key = event.key.toLocaleLowerCase()
      if (control && event.shiftKey && key === 'n') { event.preventDefault(); executeCommand('gameObject.empty'); return }
      if (control && event.shiftKey && key === 'a') { event.preventDefault(); executeCommand('component.add'); return }
      if (control && event.shiftKey && key === 'c') { event.preventDefault(); executeCommand('window.console'); return }
      if (control && event.shiftKey && key === 'b') { event.preventDefault(); executeCommand('file.build'); return }
      if (control && key === 'n') { event.preventDefault(); executeCommand('file.newScene'); return }
      if (control && key === 'o') { event.preventDefault(); executeCommand('file.openProject'); return }
      if (control && key === 's') { event.preventDefault(); executeCommand(event.shiftKey ? 'file.saveSceneAs' : 'file.saveScene'); return }
      if (control && key === 'z') { event.preventDefault(); executeCommand(event.shiftKey ? 'edit.redo' : 'edit.undo'); return }
      if (control && key === 'y') { event.preventDefault(); executeCommand('edit.redo'); return }
      if (control && key === 'x') { event.preventDefault(); executeCommand('edit.cut'); return }
      if (control && key === 'c') { event.preventDefault(); executeCommand('edit.copy'); return }
      if (control && key === 'v') { event.preventDefault(); executeCommand('edit.paste'); return }
      if (control && key === 'd') { event.preventDefault(); executeCommand('edit.duplicate'); return }
      if (control && key === 'r') { event.preventDefault(); executeCommand('assets.refresh'); return }
      if (control && key === '1') { event.preventDefault(); executeCommand('window.scene'); return }
      if (control && key === '2') { event.preventDefault(); executeCommand('window.game'); return }
      if (event.key === 'Delete') { event.preventDefault(); executeCommand('edit.delete'); return }
      if (event.key === 'F5') {
        event.preventDefault()
        if (project.runtimeMode === 'edit' ? project.capabilities.canStartPlay : project.capabilities.canStop) {
          setRuntimeMode(project.runtimeMode === 'edit' ? 'play' : 'edit')
        }
        return
      }
      if (key === 'f') { executeCommand('viewport.focusSelection'); return }
      const tool = ({ w: 'move', e: 'rotate', r: 'scale' } satisfies Partial<Record<string, TransformTool>>)[key]
      if (tool && project.capabilities.editing) controller.setTool(tool)
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [controller, executeCommand, paletteOpen, project.capabilities, project.runtimeMode, setRuntimeMode])

  return <div className={`editor-shell runtime-${project.runtimeMode}`}>
    <MenuBar projectName={project.projectName} sceneName={project.activeSceneName} sceneDirty={project.sceneDirty} disabledCommands={disabledCommands} onCommand={executeCommand} />
    <MainToolbar runtimeMode={project.runtimeMode} capabilities={project.capabilities} tool={controller.state.tool} orientationMode={controller.state.orientationMode} snappingEnabled={project.viewport.snappingEnabled} onRuntimeMode={setRuntimeMode} onStep={() => void controller.invoke('runtime.step', {})} onTool={controller.setTool} onOrientationMode={controller.setOrientationMode} onSnapping={(enabled) => void controller.invoke('viewport.setSnapping', { enabled })} onCommandPalette={() => setPaletteOpen(true)} />
    {controller.state.error && <div className="host-error-banner"><span>{controller.state.error}</span><button type="button" onClick={controller.dismissError}>Dismiss</button><button type="button" onClick={controller.reconnect}>Retry</button></div>}
    <DockWorkspace controller={controller} dock={dock} />
    <StatusBar controller={controller} onShowConsole={() => dock.show('console', 'bottom')} />
    <CommandPalette open={paletteOpen} disabledCommands={disabledCommands} onClose={() => setPaletteOpen(false)} onCommand={executeCommand} />
    <WorkflowDialogs request={workflow} controller={controller} onRequest={setWorkflow} onClose={() => setWorkflow(undefined)} />
    {project.document.pendingRecovery && <div className="modal-backdrop"><div className="command-palette" role="dialog" aria-modal="true" aria-label="Scene recovery"><div className="command-input"><strong>Recover autosaved scene?</strong></div><div className="command-results"><p>A recovery snapshot is available for the current scene.</p><button type="button" onClick={() => void controller.invoke('document.resolveRecovery', { decision: 'restore' })}>Restore</button><button type="button" onClick={() => void controller.invoke('document.resolveRecovery', { decision: 'discard' })}>Discard Recovery</button></div></div></div>}
    {project.document.pendingSwitch && <div className="modal-backdrop"><div className="command-palette" role="dialog" aria-modal="true" aria-label="Unsaved scene action"><div className="command-input"><strong>Save changes before {project.document.pendingSwitch}?</strong></div><div className="command-results"><button type="button" onClick={() => void controller.invoke('document.resolvePendingSwitch', { decision: 'save' })}>Save and Continue</button><button type="button" onClick={() => void controller.invoke('document.resolvePendingSwitch', { decision: 'discard' })}>Discard and Continue</button><button type="button" onClick={() => void controller.invoke('document.resolvePendingSwitch', { decision: 'cancel' })}>Cancel</button></div></div></div>}
    {project.document.closeConfirmation && <div className="modal-backdrop"><div className="command-palette" role="dialog" aria-modal="true" aria-label="Unsaved scene"><div className="command-input"><strong>Save changes before closing?</strong></div><div className="command-results"><p>The current scene has unsaved changes.</p><button type="button" onClick={() => void controller.invoke('document.resolveClose', { decision: 'save' })}>Save and Exit</button><button type="button" onClick={() => void controller.invoke('document.resolveClose', { decision: 'discard' })}>Discard and Exit</button><button type="button" onClick={() => void controller.invoke('document.resolveClose', { decision: 'cancel' })}>Cancel</button></div></div></div>}
  </div>
}
