import type { EditorController } from '../state/useEditorState'
import { Icon } from './Icon'

export function StatusBar({ controller, onShowConsole }: { controller: EditorController; onShowConsole(): void }) {
  const { state } = controller
  const warnings = state.project.console.filter((entry) => entry.level === 'warning').length
  const errors = state.project.console.filter((entry) => entry.level === 'error').length
  return (
    <footer className="status-bar">
      <span className={`connection-status ${state.connected ? 'connected' : 'disconnected'}`}>
        <i />{state.connected ? 'Engine connected' : state.bridgeAvailable ? 'Engine reconnecting' : 'Host disconnected'}
      </span>
      <span className="status-separator" />
      <span>{state.project.activeSceneName || 'No scene open'}{state.project.sceneDirty ? ' · Unsaved changes' : ''}</span>
      {state.project.build.active && <><span className="status-separator" /><span className="build-status"><Icon name="build" /> {state.project.build.status ?? 'Building…'}</span></>}
      <span className="status-spacer" />
      <button type="button" onClick={onShowConsole}><Icon name="warning" /> {warnings}</button>
      <button type="button" onClick={onShowConsole}><Icon name="error" /> {errors}</button>
      <span className="status-separator" />
      <span>{state.project.selection.entityIds.length} selected</span>
    </footer>
  )
}
