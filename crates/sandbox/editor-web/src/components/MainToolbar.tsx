import type { CapabilitiesSnapshot, OrientationMode, RuntimeMode, TransformTool } from '../bridge/protocol'
import { Icon, type IconName } from './Icon'

type SupportedTransformTool = Extract<TransformTool, 'move' | 'rotate' | 'scale'>

const tools: { id: SupportedTransformTool; icon: IconName; title: string; shortcut: string }[] = [
  { id: 'move', icon: 'move', title: 'Move Tool', shortcut: 'W' },
  { id: 'rotate', icon: 'rotate', title: 'Rotate Tool', shortcut: 'E' },
  { id: 'scale', icon: 'scale', title: 'Scale Tool', shortcut: 'R' },
]

interface MainToolbarProps {
  runtimeMode: RuntimeMode
  capabilities: CapabilitiesSnapshot
  tool: TransformTool
  orientationMode: OrientationMode
  snappingEnabled: boolean
  onRuntimeMode(mode: RuntimeMode): void
  onStep(): void
  onTool(tool: TransformTool): void
  onOrientationMode(mode: OrientationMode): void
  onSnapping(enabled: boolean): void
  onCommandPalette(): void
}

export function MainToolbar(props: MainToolbarProps) {
  const playing = props.runtimeMode !== 'edit'
  return (
    <div className={`main-toolbar ${playing ? 'is-playing' : ''}`}>
      <div className="tool-group transform-tools" role="toolbar" aria-label="Transform tools">
        {tools.map((tool) => <button className={props.tool === tool.id ? 'tool-button active' : 'tool-button'} type="button" key={tool.id} title={`${tool.title} (${tool.shortcut})`} disabled={!props.capabilities.editing} onClick={() => props.onTool(tool.id)}><Icon name={tool.icon} /></button>)}
      </div>
      <div className="tool-divider" />
      <div className="tool-group mode-selectors" role="group" aria-label="Transform orientation">
        <button className={props.orientationMode === 'global' ? 'segmented-text active' : 'segmented-text'} type="button" disabled={!props.capabilities.editing} onClick={() => props.onOrientationMode('global')}>Global</button>
        <button className={props.orientationMode === 'local' ? 'segmented-text active' : 'segmented-text'} type="button" disabled={!props.capabilities.editing} onClick={() => props.onOrientationMode('local')}>Local</button>
      </div>
      <div className="tool-divider" />
      <button className={props.snappingEnabled ? 'tool-button active' : 'tool-button'} type="button" title={props.snappingEnabled ? 'Disable transform snapping' : 'Enable transform snapping'} aria-pressed={props.snappingEnabled} disabled={!props.capabilities.editing} onClick={() => props.onSnapping(!props.snappingEnabled)}><Icon name="snap" /></button>
      <div className="play-controls" role="toolbar" aria-label="Play controls">
        <button className={playing ? 'play-button active' : 'play-button'} type="button" title={playing ? 'Stop' : 'Play (F5)'} disabled={playing ? !props.capabilities.canStop : !props.capabilities.canStartPlay} onClick={() => props.onRuntimeMode(playing ? 'edit' : 'play')}><Icon name="play" /></button>
        <button className={props.runtimeMode === 'paused' ? 'play-button active' : 'play-button'} type="button" title={props.runtimeMode === 'paused' ? 'Resume' : 'Pause'} disabled={props.runtimeMode === 'edit' || (!props.capabilities.canPause && !props.capabilities.canResume)} onClick={() => props.onRuntimeMode(props.runtimeMode === 'paused' ? 'play' : 'paused')}><Icon name="pause" /></button>
        <button className="play-button" type="button" title="Step" disabled={!props.capabilities.canStep} onClick={props.onStep}><Icon name="step" /></button>
      </div>
      <div className="toolbar-spacer" />
      <button className="command-search" type="button" onClick={props.onCommandPalette}><Icon name="search" /><span>Search commands</span><kbd>Ctrl K</kbd></button>
    </div>
  )
}
