import { useMemo } from 'react'
import type { EditorController } from '../state/useEditorState'
import { Icon } from '../components/Icon'

export function ProfilerPanel({ controller }: { controller: EditorController }) {
  const performance = controller.state.project.performance
  const frames = performance.history
  const chart = useMemo(() => {
    if (frames.length < 2) return { points: '', maximum: 33.3, average: frames[0]?.frameTimeMs ?? 0, minimum: frames[0]?.frameTimeMs ?? 0, peak: frames[0]?.frameTimeMs ?? 0 }
    const values = frames.map((frame) => frame.frameTimeMs)
    const maximum = Math.max(33.3, ...values)
    return {
      points: frames.map((frame, index) => `${index / (frames.length - 1) * 100},${100 - frame.frameTimeMs / maximum * 100}`).join(' '),
      maximum,
      average: values.reduce((sum, value) => sum + value, 0) / values.length,
      minimum: Math.min(...values),
      peak: Math.max(...values),
    }
  }, [frames])
  const current = performance.current
  return <div className="profiler-panel panel-column">
    <div className="profiler-toolbar profiler-live-status"><Icon name="profiler" /><strong>Live editor sampling</strong><span>The native renderer publishes frame statistics continuously.</span><div className="profiler-toolbar-spacer" /><em>{frames.length} sampled frames</em></div>
    <div className="profiler-content live-only">
      <div className="profiler-main"><div className="profiler-chart">{chart.points ? <svg viewBox="0 0 100 100" preserveAspectRatio="none"><polyline points={chart.points} /></svg> : <div className="panel-empty"><Icon name="profiler" /><span>No frame history has been sampled yet</span></div>}<span className="chart-line line-16" style={{ top: `${100 - 16.7 / chart.maximum * 100}%` }}>16.7 ms</span><span className="chart-line line-33" style={{ top: `${Math.max(0, 100 - 33.3 / chart.maximum * 100)}%` }}>33.3 ms</span><span className="chart-maximum">Scale {chart.maximum.toFixed(1)} ms</span></div>
        <div className="profiler-summary"><div><small>Frame</small><strong>{current.frameTimeMs.toFixed(2)} ms</strong></div><div><small>Draw Calls</small><strong>{current.drawCalls}</strong></div><div><small>Triangles</small><strong>{current.triangles.toLocaleString()}</strong></div><div><small>Physics Bodies</small><strong>{current.physicsBodies}</strong></div><div><small>Animations</small><strong>{current.animationCount}</strong></div><div><small>Nav Agents</small><strong>{current.navAgents}</strong></div><div><small>Assets</small><strong>{current.assetCount}</strong></div></div>
        <div className="profiler-samples panel-scroll"><div><span>History average</span><strong>{chart.average.toFixed(2)} ms</strong></div><div><span>History minimum</span><strong>{chart.minimum.toFixed(2)} ms</strong></div><div><span>History peak</span><strong>{chart.peak.toFixed(2)} ms</strong></div></div>
      </div>
    </div>
  </div>
}
