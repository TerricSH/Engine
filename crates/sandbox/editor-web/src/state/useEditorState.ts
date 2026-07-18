import { useCallback, useEffect, useMemo, useReducer } from 'react'
import { engineBridge } from '../bridge/engineBridge'
import { reduceEditorError } from '../editorErrorState'
import {
  EDITOR_PROTOCOL_VERSION,
  type EditorTelemetryEvent,
  type EditorCommand,
  type EditorCommandMap,
  type OrientationMode,
  type ProjectSnapshot,
  type SerializedAssetId,
  type TransformTool,
} from '../bridge/protocol'
import { mergeProjectTelemetry } from '../bridge/telemetry'

const EMPTY_PROJECT: ProjectSnapshot = {
  protocolVersion: EDITOR_PROTOCOL_VERSION,
  sessionId: '',
  revision: 0,
  projectName: '',
  projectPath: '',
  activeSceneName: '',
  sceneDirty: false,
  runtimeMode: 'edit',
  hierarchy: [],
  selection: { entityIds: [], components: [] },
  clipboard: { entityRootCount: 0 },
  assets: [],
  console: [],
  buildTargets: [],
  document: {
    currentSceneId: '', currentScenePath: '', dirty: false, canUndo: false,
    canRedo: false, pendingRecovery: false, closeConfirmation: false, scenes: [],
  },
  workspace: { reactLayout: '' },
  viewport: {
    sceneCamera: { pitch: 20, yaw: 45, distance: 10, target: [0, 0, 0], orthographic: false, speed: 5 },
    gizmosVisible: true,
    snappingEnabled: false,
  },
  catalog: { components: [], entityTemplates: [], verifiedScriptClasses: [] },
  assetBrowser: { query: '', folder: '', kindFilter: 'All', view: 'grid', page: 0, pageSize: 0, pageCount: 0, total: 0, visibleAssetIds: [], folders: [] },
  material: { parameters: [], writable: false },
  animation: { availableSkeletons: [], availableClips: [], playbackTime: 0, duration: 0, playing: false, looping: false, speed: 1, events: [] },
  build: { active: false, cancellable: false, output: '', packageVersion: '', packageOutputRoot: '' },
  settings: {
    windowTitle: '', windowWidth: 1600, windowHeight: 900,
    sceneSettings: {
      active_camera: null, default_render_layer: 'Default', fixed_timestep_seconds: 1 / 60,
      gravity: [0, -9.81, 0], ambient: [0.03, 0.03, 0.03, 1],
      environment_map: null, tone_mapping: 'Aces',
      pass_graph_config: { passes: [], enabled: false, output_mode: 'HdrThenToneMap' },
    },
    cameraEntities: [],
    inputMap: { name: 'player', context: 'gameplay', actions: [] },
  },
  performance: {
    current: { frameTimeMs: 0, drawCalls: 0, triangles: 0, physicsBodies: 0, animationCount: 0, navAgents: 0, assetCount: 0 },
    history: [],
  },
  capabilities: {
    editing: false, hasSelection: false, canUndo: false, canRedo: false,
    canSave: false, canStartPlay: false, canPause: false, canResume: false,
    canStep: false, canStop: false, buildBusy: false,
  },
}

export interface EditorState {
  project: ProjectSnapshot
  bridgeAvailable: boolean
  connected: boolean
  loading: boolean
  error?: string
  tool: TransformTool
  orientationMode: OrientationMode
}

type EditorAction =
  | { type: 'snapshot'; project: ProjectSnapshot }
  | { type: 'telemetry'; event: EditorTelemetryEvent }
  | { type: 'reconnected'; project: ProjectSnapshot }
  | { type: 'loading' }
  | { type: 'error'; message: string; connected: boolean }
  | { type: 'dismissError' }
  | { type: 'tool'; tool: TransformTool }
  | { type: 'orientation'; mode: OrientationMode }

function reduceEditorState(state: EditorState, action: EditorAction): EditorState {
  switch (action.type) {
    case 'snapshot': return { ...state, connected: true, loading: false, error: reduceEditorError(state.error, { type: 'snapshot' }), project: action.project }
    case 'telemetry': return { ...state, project: mergeProjectTelemetry(state.project, action.event) }
    case 'reconnected': return { ...state, connected: true, loading: false, error: reduceEditorError(state.error, { type: 'reconnectSucceeded' }), project: action.project }
    case 'loading': return { ...state, loading: true }
    case 'error': return { ...state, connected: action.connected, loading: false, error: reduceEditorError(state.error, { type: 'commandError', message: action.message }) }
    case 'dismissError': return { ...state, error: reduceEditorError(state.error, { type: 'dismissed' }) }
    case 'tool': return { ...state, tool: action.tool }
    case 'orientation': return { ...state, orientationMode: action.mode }
  }
}

export interface EditorController {
  state: EditorState
  invoke<K extends EditorCommand>(method: K, params: EditorCommandMap[K]['request']): Promise<EditorCommandMap[K]['response'] | undefined>
  reconnect(): void
  dismissError(): void
  setTool(tool: TransformTool): void
  setOrientationMode(mode: OrientationMode): void
  selectAsset(assetId?: SerializedAssetId): void
}

export function useEditorState(): EditorController {
  const [state, dispatch] = useReducer(reduceEditorState, {
    project: EMPTY_PROJECT,
    bridgeAvailable: engineBridge.available,
    connected: engineBridge.connected,
    loading: engineBridge.available,
    error: engineBridge.available ? undefined : 'Native engine bridge is unavailable',
    tool: 'move',
    orientationMode: 'global',
  })

  const invoke = useCallback(async <K extends EditorCommand>(method: K, params: EditorCommandMap[K]['request']) => {
    try {
      return await engineBridge.invoke(method, params)
    } catch (error) {
      dispatch({ type: 'error', message: error instanceof Error ? error.message : String(error), connected: engineBridge.connected })
      return undefined
    }
  }, [])

  const reconnect = useCallback(() => {
    if (!engineBridge.available) {
      dispatch({ type: 'error', message: 'Native engine bridge is unavailable', connected: false })
      return
    }
    dispatch({ type: 'loading' })
    void engineBridge.invoke('editor.ready', {
      protocolVersion: EDITOR_PROTOCOL_VERSION,
      clientVersion: 'engine-editor-web/0.1.0',
    }).then((project) => dispatch({ type: 'reconnected', project }))
      .catch((error: unknown) => dispatch({ type: 'error', message: error instanceof Error ? error.message : String(error), connected: engineBridge.connected }))
  }, [])

  useEffect(() => {
    const stopProject = engineBridge.subscribeProject((project) => dispatch({ type: 'snapshot', project }))
    const stopTelemetry = engineBridge.subscribeTelemetry((event) => dispatch({ type: 'telemetry', event }))
    const stopFault = engineBridge.subscribeFault((message) => dispatch({ type: 'error', message, connected: engineBridge.connected }))
    reconnect()
    return () => { stopProject(); stopTelemetry(); stopFault() }
  }, [reconnect])

  return useMemo(() => ({
    state,
    invoke,
    reconnect,
    dismissError() { dispatch({ type: 'dismissError' }) },
    setTool(tool: TransformTool) {
      dispatch({ type: 'tool', tool })
      void invoke('viewport.setTool', { mode: tool })
    },
    setOrientationMode(mode: OrientationMode) {
      dispatch({ type: 'orientation', mode })
      void invoke('viewport.setOrientationMode', { mode })
    },
    selectAsset(assetId?: SerializedAssetId) {
      void invoke('assets.select', { assetId })
    },
  }), [invoke, reconnect, state])
}
