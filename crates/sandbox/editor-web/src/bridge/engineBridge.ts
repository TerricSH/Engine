import {
  EDITOR_PROTOCOL,
  EDITOR_PROTOCOL_VERSION,
  type BridgeRequest,
  type BridgeResponse,
  type EditorCommand,
  type EditorCommandMap,
  type EditorTelemetryEvent,
  type NativeMessage,
  type ProjectChangedEvent,
  type ProjectSnapshot,
  type UiDockZone,
  type UiOpenPanelEvent,
  type UiPanelId,
} from './protocol'
import { isCompleteEditorTelemetry, telemetryMatchesAuthoritativeSnapshot } from './telemetry'

const RESPONSE_TIMEOUT_MS = 30_000

interface NativeIpcTransport { postMessage(message: string): void }

declare global {
  interface Window {
    ipc?: NativeIpcTransport
    __ENGINE_EDITOR_RECEIVE__?: (message: NativeMessage | string) => void
  }
}

type PendingRequest = {
  method: EditorCommand
  resolve(value: unknown): void
  reject(reason: Error): void
  timeoutId: number
}
type ProjectListener = (snapshot: ProjectSnapshot) => void
type TelemetryListener = (event: EditorTelemetryEvent) => void
type UiOpenPanelListener = (request: UiOpenPanelEvent['params']) => void
type FaultListener = (message: string) => void
type NotificationCommand = 'viewport.bounds' | 'viewport.input'

export interface EngineBridge {
  readonly available: boolean
  readonly connected: boolean
  invoke<K extends EditorCommand>(method: K, params: EditorCommandMap[K]['request']): Promise<EditorCommandMap[K]['response']>
  notify<K extends NotificationCommand>(method: K, params: EditorCommandMap[K]['request']): void
  subscribeProject(listener: ProjectListener): () => void
  subscribeTelemetry(listener: TelemetryListener): () => void
  subscribeUiOpenPanel(listener: UiOpenPanelListener): () => void
  subscribeFault(listener: FaultListener): () => void
}

function getTransport(): NativeIpcTransport | undefined { return window.ipc }

function createRequestId(): string {
  return 'randomUUID' in crypto
    ? crypto.randomUUID()
    : `editor-${Date.now()}-${Math.random().toString(16).slice(2)}`
}

function isSafeCounter(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

const UI_PANEL_IDS = new Set<UiPanelId>(['hierarchy', 'scene', 'game', 'inspector', 'project', 'console', 'material', 'animation', 'profiler', 'terrain', 'build', 'settings'])
const UI_DOCK_ZONES = new Set<UiDockZone>(['left', 'center', 'right', 'bottom'])

const SNAPSHOT_KEYS: readonly (keyof ProjectSnapshot)[] = [
  'protocolVersion', 'sessionId', 'revision', 'projectName', 'projectPath',
  'activeSceneName', 'sceneDirty', 'runtimeMode', 'hierarchy', 'selection',
  'assets', 'console', 'buildTargets', 'document', 'workspace', 'viewport', 'catalog',
  'assetBrowser', 'material', 'animation', 'build', 'settings', 'performance', 'terrain',
  'capabilities',
]

function isCompleteProjectSnapshot(value: unknown): value is ProjectSnapshot {
  if (!value || typeof value !== 'object') return false
  const snapshot = value as Record<string, unknown>
  if (!SNAPSHOT_KEYS.every((key) => Object.hasOwn(snapshot, key))) return false
  return snapshot.protocolVersion === EDITOR_PROTOCOL_VERSION
    && typeof snapshot.sessionId === 'string'
    && isSafeCounter(snapshot.revision)
    && typeof snapshot.projectName === 'string'
    && typeof snapshot.projectPath === 'string'
    && typeof snapshot.activeSceneName === 'string'
    && typeof snapshot.sceneDirty === 'boolean'
    && (snapshot.runtimeMode === 'edit' || snapshot.runtimeMode === 'play' || snapshot.runtimeMode === 'paused')
    && Array.isArray(snapshot.hierarchy)
    && Boolean(snapshot.selection && typeof snapshot.selection === 'object')
    && Array.isArray(snapshot.assets)
    && Array.isArray(snapshot.console)
    && Array.isArray(snapshot.buildTargets)
    && Boolean(snapshot.document && typeof snapshot.document === 'object')
    && Boolean(snapshot.workspace && typeof snapshot.workspace === 'object' && typeof (snapshot.workspace as Record<string, unknown>).reactLayout === 'string')
    && Boolean(snapshot.viewport && typeof snapshot.viewport === 'object')
    && Boolean(snapshot.catalog && typeof snapshot.catalog === 'object')
    && Boolean(snapshot.assetBrowser && typeof snapshot.assetBrowser === 'object')
    && Boolean(snapshot.material && typeof snapshot.material === 'object')
    && Boolean(snapshot.animation && typeof snapshot.animation === 'object')
    && Boolean(snapshot.build && typeof snapshot.build === 'object')
    && Boolean(snapshot.settings && typeof snapshot.settings === 'object')
    && Boolean(snapshot.performance && typeof snapshot.performance === 'object')
    && Boolean(snapshot.capabilities && typeof snapshot.capabilities === 'object')
}

export function createEngineBridge(): EngineBridge {
  const pending = new Map<string, PendingRequest>()
  const projectListeners = new Set<ProjectListener>()
  const telemetryListeners = new Set<TelemetryListener>()
  const uiOpenPanelListeners = new Set<UiOpenPanelListener>()
  const faultListeners = new Set<FaultListener>()
  let sessionId: string | undefined
  let revision: number | undefined
  let authoritativeSnapshotRevision: number | undefined
  let lastSequence: number | undefined
  let commandQueue: Promise<unknown> = Promise.resolve()

  const fault = (message: string) => {
    console.error(message)
    faultListeners.forEach((listener) => listener(message))
  }

  const rejectAll = (message: string) => {
    const error = new Error(message)
    pending.forEach((request) => {
      window.clearTimeout(request.timeoutId)
      request.reject(error)
    })
    pending.clear()
  }

  const invalidateSession = (message: string) => {
    rejectAll(message)
    sessionId = undefined
    revision = undefined
    authoritativeSnapshotRevision = undefined
    lastSequence = undefined
    fault(message)
  }

  const acceptSessionMessage = (messageSession: string, messageRevision: number): boolean => {
    if (!isSafeCounter(messageRevision)) {
      invalidateSession('Engine IPC rejected a non-safe revision counter; Retry to reconnect')
      return false
    }
    if (sessionId !== undefined && messageSession !== sessionId) {
      invalidateSession('The native editor session changed; Retry to reconnect')
      return false
    }
    if (revision !== undefined && messageRevision < revision) return false
    revision = messageRevision
    return true
  }

  const deliverResponse = (message: BridgeResponse) => {
    const request = pending.get(message.id)
    if (!request) {
      // Fire-and-forget viewport notifications still receive acknowledgements.
      if (sessionId === undefined) return
      if (!isSafeCounter(message.revision) || message.sessionId !== sessionId) {
        invalidateSession('A viewport acknowledgement did not match the active session; Retry to reconnect')
      } else if (message.error) {
        const mustReconnect = message.error.code === 'staleRevision'
          || message.error.code === 'protocolMismatch'
          || message.revision !== revision
        if (mustReconnect) invalidateSession(`${message.error.code}: ${message.error.message}; Retry to reconnect`)
        else fault(`${message.error.code}: ${message.error.message}`)
      }
      return
    }
    window.clearTimeout(request.timeoutId)
    pending.delete(message.id)

    if (request.method === 'editor.ready') {
      if (message.error) {
        request.reject(new Error(`${message.error.code}: ${message.error.message}`))
        return
      }
      if (!isSafeCounter(message.revision) || !isCompleteProjectSnapshot(message.result)) {
        request.reject(new Error('The editor handshake returned an incomplete snapshot'))
        return
      }
      if (message.result.sessionId !== message.sessionId || message.result.revision !== message.revision) {
        request.reject(new Error('The editor handshake session metadata is inconsistent'))
        return
      }
      sessionId = message.sessionId
      revision = message.revision
      authoritativeSnapshotRevision = message.revision
      lastSequence = undefined
      request.resolve(message.result)
      return
    }

    if (!isSafeCounter(message.revision) || message.sessionId !== sessionId) {
      request.reject(new Error('The editor response belongs to an invalid session'))
      invalidateSession('The editor response did not match the active session; Retry to reconnect')
      return
    }
    if (message.error) {
      const mustReconnect = message.error.code === 'staleRevision'
        || message.error.code === 'protocolMismatch'
        || message.revision !== revision
      request.reject(new Error(`${message.error.code}: ${message.error.message}`))
      if (mustReconnect) invalidateSession(`${message.error.code}: ${message.error.message}; Retry to reconnect`)
      return
    }
    if (!acceptSessionMessage(message.sessionId, message.revision)) {
      request.reject(new Error('The editor response belongs to a stale revision'))
      return
    }
    request.resolve(message.result)
  }

  const deliverProjectChanged = (message: ProjectChangedEvent) => {
    if (!sessionId) return
    if (message.sessionId !== sessionId) {
      invalidateSession('project.changed belongs to a different editor session; Retry to reconnect')
      return
    }
    if (!isSafeCounter(message.sequence) || !isSafeCounter(message.revision)) {
      invalidateSession('project.changed carried an unsafe counter; Retry to reconnect')
      return
    }
    if (lastSequence !== undefined && message.sequence <= lastSequence) return
    if (!isCompleteProjectSnapshot(message.params)
      || message.params.sessionId !== message.sessionId
      || message.params.revision !== message.revision) {
      invalidateSession('project.changed was incomplete or inconsistent; Retry to reconnect')
      return
    }
    // Events are complete snapshots, so a sequence gap is safe: the newest
    // accepted event replaces all UI state instead of applying partial deltas.
    if (!acceptSessionMessage(message.sessionId, message.revision)) return
    authoritativeSnapshotRevision = message.revision
    lastSequence = message.sequence
    projectListeners.forEach((listener) => listener(message.params))
  }

  const deliverUiOpenPanel = (message: UiOpenPanelEvent) => {
    if (!sessionId) return
    if (message.sessionId !== sessionId) {
      invalidateSession('ui.openPanel belongs to a different editor session; Retry to reconnect')
      return
    }
    if (!isSafeCounter(message.sequence) || !isSafeCounter(message.revision)) {
      invalidateSession('ui.openPanel carried an unsafe counter; Retry to reconnect')
      return
    }
    if (lastSequence !== undefined && message.sequence <= lastSequence) return
    if (!message.params || !UI_PANEL_IDS.has(message.params.panel) || !UI_DOCK_ZONES.has(message.params.preferredZone)) {
      invalidateSession('ui.openPanel carried an invalid panel request; Retry to reconnect')
      return
    }
    if (!acceptSessionMessage(message.sessionId, message.revision)) return
    lastSequence = message.sequence
    uiOpenPanelListeners.forEach((listener) => listener(message.params))
  }

  const deliverTelemetry = (message: EditorTelemetryEvent) => {
    if (!sessionId || revision === undefined || authoritativeSnapshotRevision === undefined) return
    if (message.sessionId !== sessionId) {
      invalidateSession('editor.telemetry belongs to a different editor session; Retry to reconnect')
      return
    }
    if (!isSafeCounter(message.sequence) || !isSafeCounter(message.revision)) {
      invalidateSession('editor.telemetry carried an unsafe counter; Retry to reconnect')
      return
    }
    if (lastSequence !== undefined && message.sequence <= lastSequence) return
    // A mutation response can advance the command revision before React receives the matching
    // complete project.changed snapshot. Drop telemetry in that window instead of merging it
    // into an older authoritative snapshot or disconnecting a healthy session.
    if (!telemetryMatchesAuthoritativeSnapshot(authoritativeSnapshotRevision, message.revision)) return
    if (!isCompleteEditorTelemetry(message.params)) {
      invalidateSession('editor.telemetry omitted or corrupted a complete telemetry domain; Retry to reconnect')
      return
    }
    // A sequence gap is safe at the authoritative snapshot revision: every telemetry event
    // replaces all three high-frequency domains. Newer telemetry waits for project.changed.
    lastSequence = message.sequence
    telemetryListeners.forEach((listener) => listener(message))
  }

  const deliver = (rawMessage: NativeMessage | string) => {
    let message: NativeMessage
    try {
      message = typeof rawMessage === 'string' ? JSON.parse(rawMessage) as NativeMessage : rawMessage
    } catch {
      fault('Engine delivered malformed IPC JSON')
      return
    }
    if (!message || typeof message !== 'object' || message.protocol !== EDITOR_PROTOCOL) {
      fault('Engine delivered a message for an incompatible editor protocol')
      return
    }
    if ('id' in message) deliverResponse(message)
    else if (message.event === 'project.changed') deliverProjectChanged(message)
    else if (message.event === 'editor.telemetry') deliverTelemetry(message)
    else if (message.event === 'ui.openPanel') deliverUiOpenPanel(message)
    else fault('Engine delivered an unknown editor event')
  }

  window.__ENGINE_EDITOR_RECEIVE__ = deliver

  const post = <K extends EditorCommand>(method: K, params: EditorCommandMap[K]['request'], trackResponse: boolean) => {
    const transport = getTransport()
    if (!transport) return Promise.reject(new Error('Native engine IPC is unavailable. Start the editor through the Rust host.'))
    if (method !== 'editor.ready' && (!sessionId || revision === undefined)) {
      return Promise.reject(new Error('The editor handshake has not completed'))
    }

    const id = createRequestId()
    const request: BridgeRequest<K> = {
      protocol: EDITOR_PROTOCOL,
      id,
      method,
      params,
      ...(method === 'editor.ready' ? {} : { sessionId, baseRevision: revision }),
    }

    if (!trackResponse) {
      transport.postMessage(JSON.stringify(request))
      return Promise.resolve(undefined)
    }
    return new Promise<EditorCommandMap[K]['response']>((resolve, reject) => {
      const timeoutId = window.setTimeout(() => {
        pending.delete(id)
        reject(new Error(`Engine IPC request timed out: ${method}`))
      }, RESPONSE_TIMEOUT_MS)
      pending.set(id, { method, resolve: resolve as (value: unknown) => void, reject, timeoutId })
      transport.postMessage(JSON.stringify(request))
    })
  }

  return {
    get available() { return Boolean(getTransport()) },
    get connected() { return sessionId !== undefined && revision !== undefined },
    invoke<K extends EditorCommand>(method: K, params: EditorCommandMap[K]['request']) {
      const run = () => post(method, params, true) as Promise<EditorCommandMap[K]['response']>
      if (method === 'editor.ready') return run()
      const queued = commandQueue.then(run, run)
      commandQueue = queued.catch(() => undefined)
      return queued
    },
    notify<K extends NotificationCommand>(method: K, params: EditorCommandMap[K]['request']) {
      if (!sessionId || revision === undefined) return
      void post(method, params, false).catch((error: unknown) => fault(error instanceof Error ? error.message : String(error)))
    },
    subscribeProject(listener: ProjectListener) {
      projectListeners.add(listener)
      return () => projectListeners.delete(listener)
    },
    subscribeTelemetry(listener: TelemetryListener) {
      telemetryListeners.add(listener)
      return () => telemetryListeners.delete(listener)
    },
    subscribeUiOpenPanel(listener: UiOpenPanelListener) {
      uiOpenPanelListeners.add(listener)
      return () => uiOpenPanelListeners.delete(listener)
    },
    subscribeFault(listener: FaultListener) {
      faultListeners.add(listener)
      return () => faultListeners.delete(listener)
    },
  }
}

export const engineBridge = createEngineBridge()
