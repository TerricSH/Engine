import type { EditorController } from '../state/useEditorState'
import { Icon } from '../components/Icon'

const STANDARD_SPEEDS = [0.25, 0.5, 1, 1.5, 2]

export function AnimationPanel({ controller }: { controller: EditorController }) {
  const animation = controller.state.project.animation
  const duration = Math.max(animation.duration, 0.001)
  return <div className="animation-panel panel-column">
    <div className="animation-toolbar">
      <select aria-label="Skeleton" value={animation.selectedSkeleton ?? ''} onChange={(event) => void controller.invoke('animation.setState', { skeleton: event.target.value || null })}><option value="">No skeleton</option>{animation.availableSkeletons.map((skeleton) => <option key={skeleton}>{skeleton}</option>)}</select>
      <select aria-label="Animation clip" value={animation.selectedClip ?? ''} onChange={(event) => void controller.invoke('animation.setState', { clip: event.target.value || null, time: 0 })}><option value="">No clip</option>{animation.availableClips.map((clip) => <option key={clip}>{clip}</option>)}</select>
      <button type="button" title={animation.playing ? 'Pause preview' : 'Play preview'} className={animation.playing ? 'active' : ''} disabled={!animation.selectedClip} onClick={() => void controller.invoke('animation.setState', { playing: !animation.playing })}><Icon name={animation.playing ? 'pause' : 'play'} /></button>
      <label><input type="checkbox" checked={animation.looping} onChange={(event) => void controller.invoke('animation.setState', { looping: event.target.checked })} /> Loop</label>
      <label className="animation-speed">Speed <select aria-label="Animation playback speed" value={animation.speed} onChange={(event) => void controller.invoke('animation.setState', { speed: Number(event.target.value) })}>{!STANDARD_SPEEDS.includes(animation.speed) && <option value={animation.speed}>{animation.speed}×</option>}{STANDARD_SPEEDS.map((speed) => <option value={speed} key={speed}>{speed}×</option>)}</select></label>
      <span className="animation-time">{animation.playbackTime.toFixed(3)} s / {animation.duration.toFixed(3)} s</span>
    </div>
    <div className="timeline-content">
      <div className="track-list panel-scroll"><div className="track-header">Animation Events</div>{animation.events.map((event, index) => <div key={`${event.name}-${index}`}><span>{event.name}</span><small>{event.time.toFixed(3)} s</small></div>)}{animation.events.length === 0 && <div className="track-empty">No events in this clip</div>}</div>
      <div className="timeline-editor panel-scroll"><div className="timeline-ruler">{Array.from({ length: 11 }, (_, index) => <span key={index} style={{ left: `${index * 10}%` }}>{(duration * index / 10).toFixed(1)}</span>)}</div><div className="timeline-playhead" style={{ left: `${Math.min(animation.playbackTime / duration * 100, 100)}%` }} />
        {animation.events.map((event, index) => <div className="timeline-track" key={`${event.name}-${index}`}><span className="keyframe" title={`${event.name} at ${event.time.toFixed(3)} s`} style={{ left: `${Math.min(event.time / duration * 100, 100)}%` }} /></div>)}
        <input aria-label="Animation playback time" className="timeline-scrubber" type="range" min={0} max={duration} step={duration / 1000} value={Math.min(animation.playbackTime, duration)} disabled={!animation.selectedClip} onChange={(event) => void controller.invoke('animation.setState', { time: Number(event.target.value) })} />
      </div>
    </div>
  </div>
}
