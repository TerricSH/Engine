export const EDITOR_PROTOCOL = 'EngineEditorIpc-v1' as const
export const EDITOR_PROTOCOL_VERSION = 1 as const

export type EntityId = string
export type AssetId = string
export interface SerializedAssetId { id: string; logical_path: string | null }
export type RuntimeMode = 'edit' | 'play' | 'paused'
export type TransformTool = 'move' | 'rotate' | 'scale'
export type OrientationMode = 'global' | 'local'
export type UiPanelId = 'hierarchy' | 'scene' | 'game' | 'inspector' | 'project' | 'console' | 'material' | 'animation' | 'profiler' | 'build' | 'settings'
export type UiDockZone = 'left' | 'center' | 'right' | 'bottom'

export interface Vec3 { x: number; y: number; z: number }
export interface ScreenRect { x: number; y: number; width: number; height: number }

export type EngineValue =
  | { Bool: boolean }
  | { Int: number }
  | { UInt: number }
  | { Float32: number }
  | { Float64: number }
  | { Str: string }
  | { Vec3: [number, number, number] }
  | { Quat: [number, number, number, number] }
  | { Color: [number, number, number, number] }
  | { Asset: SerializedAssetId }
  | { Entity: string }
  | { Enum: string }
  | { List: EngineValue[] }
  | { Map: Record<string, EngineValue> }

export interface TransformSnapshot { position: Vec3; rotationEuler: Vec3; scale: Vec3 }
export interface HierarchyNode { id: EntityId; name: string; enabled: boolean; expanded: boolean; prefab?: AssetId; children: HierarchyNode[] }
export interface ComponentField {
  path: string
  label: string
  value: unknown
  valueType: 'boolean' | 'number' | 'string' | 'vec3' | 'vec4' | 'color' | 'asset' | 'enum' | 'list' | 'map'
  engineValue: EngineValue
  acceptedAssetKinds: AssetKind[]
}
export interface ComponentSnapshot {
  typeId: string
  displayName: string
  enabled: boolean
  removable: boolean
  resettable: boolean
  removeBlockedReason?: string
  fields: ComponentField[]
}
export interface SelectionSnapshot {
  entityIds: EntityId[]
  activeEntityId?: EntityId
  displayName?: string
  active?: boolean
  transform?: TransformSnapshot
  components: ComponentSnapshot[]
}
export interface ClipboardSnapshot {
  entityRootCount: number
  componentType?: string
}

export type AssetKind = 'scene' | 'prefab' | 'model' | 'material' | 'texture' | 'audio' | 'navmesh' | 'script' | 'shader' | 'other'
export interface AssetSnapshot {
  id: AssetId
  assetId: SerializedAssetId
  name: string
  path: string
  kind: AssetKind
  loaded: boolean
  cooked: boolean
  manifestDeclared: boolean
}
export type ConsoleLevel = 'info' | 'warning' | 'error'
export interface ConsoleEntry {
  id: string
  timestamp: string
  level: ConsoleLevel
  source: string
  code: string
  message: string
  path?: string
  entity?: string
  suggestedAction?: string
}
export interface BuildTargetSnapshot { id: string; name: string; platform: string; architecture: string; active: boolean }

export interface SceneDocumentSnapshot { id: string; path: string; startup: boolean; current: boolean }
export interface DocumentSnapshot {
  currentSceneId: string
  currentScenePath: string
  dirty: boolean
  canUndo: boolean
  canRedo: boolean
  status?: string
  pendingSwitch?: string
  pendingRecovery: boolean
  closeConfirmation: boolean
  scenes: SceneDocumentSnapshot[]
}
export interface WorkspaceSnapshot {
  reactLayout: string
}
export interface SceneCameraSnapshot {
  pitch: number
  yaw: number
  distance: number
  target: [number, number, number]
  orthographic: boolean
  speed: number
}
export interface ViewportSnapshot {
  sceneCamera: SceneCameraSnapshot
  gizmosVisible: boolean
  snappingEnabled: boolean
}
export interface ComponentDescriptor {
  typeId: string
  displayName: string
  category: string
  removable: boolean
  requiredComponents: string[]
}
export interface EntityTemplateSnapshot { id: string; displayName: string; category: string; componentTypes: string[] }
export interface ScriptClassSnapshot { assemblyId: string; className: string }
export interface CatalogSnapshot {
  components: ComponentDescriptor[]
  entityTemplates: EntityTemplateSnapshot[]
  verifiedScriptClasses: ScriptClassSnapshot[]
}
export interface AssetFolderSnapshot { path: string; name: string; depth: number; directAssetCount: number }
export interface AssetBrowserSnapshot {
  query: string
  folder: string
  kindFilter: string
  view: 'grid' | 'list'
  page: number
  pageSize: number
  pageCount: number
  total: number
  visibleAssetIds: string[]
  folders: AssetFolderSnapshot[]
  selectedAsset?: string
}
export interface MaterialParameterSnapshot { name: string; kind: 'float' | 'color' | 'texture'; value: unknown }
export interface MaterialSnapshot {
  selectedMaterial?: string
  parameters: MaterialParameterSnapshot[]
  writable: boolean
  readOnlyReason?: string
  saveStatus?: string
}
export interface AnimationEventSnapshot { time: number; name: string }
export interface AnimationSnapshot {
  availableSkeletons: string[]
  availableClips: string[]
  selectedSkeleton?: string
  selectedClip?: string
  playbackTime: number
  duration: number
  playing: boolean
  looping: boolean
  speed: number
  events: AnimationEventSnapshot[]
}
export interface BuildSnapshot {
  active: boolean
  cancellable: boolean
  status?: string
  output: string
  packageVersion: string
  packageOutputRoot: string
}
export interface BackgroundOperationSnapshot {
  id: number
  label: string
  state: 'running' | 'succeeded' | 'committedWithWarning' | 'failed'
  error?: string
}
export interface SceneSettingsSnapshot {
  active_camera: string | null
  default_render_layer: string
  fixed_timestep_seconds: number
  gravity: [number, number, number] | null
  ambient: [number, number, number, number]
  environment_map: SerializedAssetId | null
  tone_mapping: 'Aces' | 'Reinhard' | 'None'
  pass_graph_config: {
    passes: { kind: string; enabled: boolean }[]
    enabled: boolean
    output_mode: 'HdrThenToneMap' | 'DirectToSwapchain'
  }
}
export type InputModifierSnapshot = 'None' | 'Invert' | { Deadzone: number } | { Scale: number }
export type InputValueSnapshot = { Bool: boolean } | { Float: number } | { Vec2: [number, number] }
export interface InputBindingSnapshot {
  device: 'KeyboardMouse' | 'Gamepad' | 'Touch'
  action: string
  keys: string[]
  gamepad_button: string | null
  gamepad_axis: string | null
  modifier: InputModifierSnapshot
}
export interface InputActionSnapshot {
  name: string
  bindings: InputBindingSnapshot[]
  value_type: 'Digital' | 'Analog1D' | 'Analog2D'
  current_value: InputValueSnapshot
}
export interface InputActionMapSnapshot { name: string; actions: InputActionSnapshot[]; context: string }
export interface SettingsSnapshot {
  windowTitle: string
  windowWidth: number
  windowHeight: number
  sceneSettings: SceneSettingsSnapshot
  cameraEntities: { id: string; name: string }[]
  inputMap: InputActionMapSnapshot
}
export interface FrameStatsSnapshot {
  frameTimeMs: number
  drawCalls: number
  triangles: number
  physicsBodies: number
  animationCount: number
  navAgents: number
  assetCount: number
}
export interface PerformanceSnapshot { current: FrameStatsSnapshot; history: FrameStatsSnapshot[] }
export interface CapabilitiesSnapshot {
  editing: boolean
  hasSelection: boolean
  canUndo: boolean
  canRedo: boolean
  canSave: boolean
  canStartPlay: boolean
  canPause: boolean
  canResume: boolean
  canStep: boolean
  canStop: boolean
  buildBusy: boolean
}

/** Exact camelCase shape serialized by editor_app/snapshot.rs. */
export interface ProjectSnapshot {
  protocolVersion: typeof EDITOR_PROTOCOL_VERSION
  sessionId: string
  revision: number
  projectName: string
  projectPath: string
  activeSceneName: string
  sceneDirty: boolean
  runtimeMode: RuntimeMode
  hierarchy: HierarchyNode[]
  selection: SelectionSnapshot
  clipboard: ClipboardSnapshot
  assets: AssetSnapshot[]
  console: ConsoleEntry[]
  buildTargets: BuildTargetSnapshot[]
  document: DocumentSnapshot
  workspace: WorkspaceSnapshot
  viewport: ViewportSnapshot
  catalog: CatalogSnapshot
  assetBrowser: AssetBrowserSnapshot
  material: MaterialSnapshot
  animation: AnimationSnapshot
  build: BuildSnapshot
  backgroundOperation?: BackgroundOperationSnapshot
  backgroundOperations?: BackgroundOperationSnapshot[]
  settings: SettingsSnapshot
  performance: PerformanceSnapshot
  capabilities: CapabilitiesSnapshot
}

export interface InputModifiers { alt: boolean; control: boolean; meta: boolean; shift: boolean }
export type ViewportInput =
  | { type: 'pointerDown' | 'pointerUp' | 'pointerMove'; pointerId: number; x: number; y: number; button: number; buttons: number; modifiers: InputModifiers }
  | { type: 'pointerCancel'; pointerId: number }
  | { type: 'wheel'; x: number; y: number; deltaX: number; deltaY: number; deltaMode: number; modifiers: InputModifiers }
  | { type: 'keyDown' | 'keyUp'; key: string; code: string; repeat: boolean; modifiers: InputModifiers }
  | { type: 'focus' | 'blur' }

type Empty = Record<string, never>
type Accepted = { accepted: boolean; jobId?: number }

export interface EditorCommandMap {
  'editor.ready': { request: { protocolVersion: typeof EDITOR_PROTOCOL_VERSION; clientVersion?: string }; response: ProjectSnapshot }
  'editor.getSnapshot': { request: Empty; response: ProjectSnapshot }
  'editor.quit': { request: Empty; response: Accepted }
  'document.save': { request: Empty; response: Accepted }
  'document.open': { request: { sceneId: string }; response: Accepted }
  'document.create': { request: { sceneId: string; folder: string }; response: Accepted }
  'document.saveAs': { request: { sceneId: string }; response: Accepted }
  'document.duplicate': { request: { sourceId: string; newId: string }; response: Accepted }
  'document.rename': { request: { oldId: string; newId: string }; response: Accepted }
  'document.delete': { request: { sceneId: string; replacementStartup?: string }; response: Accepted }
  'document.setStartup': { request: { sceneId: string }; response: Accepted }
  'document.resolvePendingSwitch': { request: { decision: 'save' | 'discard' | 'cancel' }; response: Accepted }
  'document.resolveRecovery': { request: { decision: 'restore' | 'discard' }; response: Accepted }
  'document.resolveClose': { request: { decision: 'save' | 'discard' | 'cancel' }; response: Accepted }
  'scene.undo': { request: Empty; response: Accepted }
  'scene.redo': { request: Empty; response: Accepted }
  'scene.select': { request: { entityId?: EntityId; entityIds?: EntityId[] }; response: SelectionSnapshot }
  'scene.createEntity': { request: { templateId?: string; parentId?: EntityId; entityId?: EntityId }; response: { entityId: EntityId } }
  'scene.setEntityEnabled': { request: { entityId?: EntityId; entityIds?: EntityId[]; enabled: boolean }; response: Accepted }
  'scene.renameEntity': { request: { entityId: EntityId; name?: string }; response: Accepted }
  'scene.setEntityParent': { request: { entityId?: EntityId; entityIds?: EntityId[]; parent?: EntityId }; response: Accepted }
  'scene.moveEntity': { request: { entityId: EntityId; movement: 'up' | 'down' | 'first' | 'last' }; response: Accepted }
  'scene.copyEntities': { request: { entityIds: EntityId[] }; response: Accepted }
  'scene.cutEntities': { request: { entityIds: EntityId[] }; response: Accepted }
  'scene.pasteEntities': { request: { parentId?: EntityId }; response: Accepted }
  'scene.duplicateEntity': { request: { entityId?: EntityId; entityIds?: EntityId[] }; response: Accepted }
  'scene.deleteEntity': { request: { entityId?: EntityId; entityIds?: EntityId[] }; response: Accepted }
  'scene.setComponentEnabled': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string; enabled: boolean }; response: Accepted }
  'scene.setComponentField': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string; fieldName: string; value: EngineValue }; response: Accepted }
  'scene.addComponent': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string }; response: Accepted }
  'scene.resetComponent': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string }; response: Accepted }
  'scene.removeComponent': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string }; response: Accepted }
  'scene.copyComponent': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string }; response: Accepted }
  'scene.pasteComponent': { request: { entityId?: EntityId; entityIds?: EntityId[]; componentType: string }; response: Accepted }
  'scene.applySettings': { request: { settings: SceneSettingsSnapshot }; response: Accepted }
  'runtime.setMode': { request: { mode: RuntimeMode }; response: Accepted }
  'runtime.step': { request: Empty; response: Accepted }
  'viewport.bounds': { request: { viewport: 'scene' | 'game'; rect: ScreenRect; visible: boolean }; response: Accepted }
  'viewport.input': { request: { viewport: 'scene' | 'game'; event: ViewportInput }; response: Accepted }
  'viewport.setTool': { request: { mode: TransformTool }; response: Accepted }
  'viewport.setOrientationMode': { request: { mode: OrientationMode }; response: Accepted }
  'viewport.setSnapping': { request: { enabled: boolean }; response: Accepted }
  'viewport.focusSelection': { request: Empty; response: Accepted }
  'viewport.setCamera': { request: { pitch: number; yaw: number; distance: number; target: [number, number, number]; orthographic: boolean; speed: number }; response: Accepted }
  'viewport.setGizmos': { request: { visible: boolean }; response: Accepted }
  'assets.select': { request: { assetId?: SerializedAssetId }; response: Accepted }
  'assets.setBrowser': { request: { query?: string; folder?: string; kind?: string; page?: number; view?: string }; response: Accepted }
  'assets.refresh': { request: Empty; response: Accepted }
  'project.reveal': { request: Empty; response: Accepted }
  'assets.revealFolder': { request: { folder: string }; response: Accepted }
  'assets.reveal': { request: { assetId: string }; response: Accepted }
  'assets.open': { request: { assetId: string }; response: Accepted }
  'assets.import': { request: { source: string; assetId: string; assetType?: string; folder?: string }; response: Accepted }
  'assets.createFolder': { request: { folder: string }; response: Accepted }
  'assets.renameFolder': { request: { folder: string; newFolder: string }; response: Accepted }
  'assets.deleteFolder': { request: { folder: string }; response: Accepted }
  'assets.createMaterial': { request: { folder: string; name: string }; response: Accepted }
  'assets.createPrefab': { request: { assetId: string; relativeSourcePath: string; manifestName: string }; response: Accepted }
  'assets.instantiatePrefab': { request: { assetId: SerializedAssetId; parentId?: EntityId }; response: Accepted }
  'assets.unpackPrefab': { request: { entityId: EntityId; mode: 'instance' | 'completely' }; response: Accepted }
  'assets.duplicate': { request: { assetId: string }; response: Accepted }
  'assets.move': { request: { assetId: string; newSourcePath: string }; response: Accepted }
  'assets.delete': { request: { assetId: string }; response: Accepted }
  'assets.assign': { request: { assetId: string; entityId: string }; response: Accepted }
  'material.open': { request: { assetId: string }; response: Accepted }
  'material.setParameter': { request: { name: string; value: unknown }; response: Accepted }
  'material.save': { request: Empty; response: Accepted }
  'material.assign': { request: Empty; response: Accepted }
  'animation.setState': { request: { skeleton?: string | null; clip?: string | null; playing?: boolean; looping?: boolean; speed?: number; time?: number }; response: Accepted }
  'console.clear': { request: Empty; response: Accepted }
  'console.export': { request: Empty; response: Accepted }
  'build.start': { request: { targetId?: string; operation?: 'validate' | 'cookAndCompile' | 'packageWindows'; runAfterBuild?: boolean; version?: string; outputRoot?: string }; response: Accepted }
  'build.cancel': { request: Empty; response: Accepted }
  'build.run': { request: Empty; response: Accepted }
  'project.create': { request: { root: string; name?: string; withCsharp?: boolean }; response: Accepted }
  'project.open': { request: { path: string }; response: Accepted }
  'project.saveSettings': { request: { title: string; width: number; height: number }; response: Accepted }
  'settings.replaceInputMap': { request: { map: InputActionMapSnapshot }; response: Accepted }
  'settings.saveInputMap': { request: Empty; response: Accepted }
  'script.create': { request: { className: string; folder: string }; response: Accepted }
  'script.rebuild': { request: Empty; response: Accepted }
  'script.attach': { request: { entityId: string; assemblyId: string; className: string }; response: Accepted }
  'layout.persist': { request: { serializedLayout: string }; response: Accepted }
}

export type EditorCommand = keyof EditorCommandMap

export interface BridgeRequest<K extends EditorCommand = EditorCommand> {
  protocol: typeof EDITOR_PROTOCOL
  id: string
  method: K
  params: EditorCommandMap[K]['request']
  sessionId?: string
  baseRevision?: number
}
export interface BridgeError { code: string; message: string; field?: string; currentRevision?: number }
export interface BridgeResponse {
  protocol: typeof EDITOR_PROTOCOL
  id: string
  sessionId: string
  revision: number
  result?: unknown
  error?: BridgeError
}
export interface ProjectChangedEvent {
  protocol: typeof EDITOR_PROTOCOL
  sessionId: string
  sequence: number
  revision: number
  event: 'project.changed'
  params: ProjectSnapshot
}
export interface EditorTelemetry {
  performance: PerformanceSnapshot
  animation: AnimationSnapshot
  build: BuildSnapshot
}
export interface EditorTelemetryEvent {
  protocol: typeof EDITOR_PROTOCOL
  sessionId: string
  sequence: number
  revision: number
  event: 'editor.telemetry'
  params: EditorTelemetry
}
export interface UiOpenPanelEvent {
  protocol: typeof EDITOR_PROTOCOL
  sessionId: string
  sequence: number
  revision: number
  event: 'ui.openPanel'
  params: { panel: UiPanelId; preferredZone: UiDockZone }
}
export interface EditorEventMap {
  'project.changed': ProjectSnapshot
  'editor.telemetry': EditorTelemetry
  'ui.openPanel': UiOpenPanelEvent['params']
}
export type NativeMessage = BridgeResponse | ProjectChangedEvent | EditorTelemetryEvent | UiOpenPanelEvent
