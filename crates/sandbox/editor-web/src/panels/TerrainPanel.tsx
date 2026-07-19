import { useEffect, useState } from 'react'
import type { EngineValue, TerrainSnapshot } from '../bridge/protocol'
import type { EditorController } from '../state/useEditorState'

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KiB`
  return `${(bytes / (1024 * 1024)).toFixed(1)} MiB`
}

function NumericField({ label, value, onCommit, step = 'any' }: { label: string; value: number; onCommit(value: number): void; step?: string }) {
  const [draft, setDraft] = useState(String(value))
  useEffect(() => setDraft(String(value)), [value])
  const commit = () => {
    const parsed = Number(draft)
    if (Number.isFinite(parsed) && parsed !== value) onCommit(parsed)
    else setDraft(String(value))
  }
  return <label className="inspector-field"><span>{label}</span><input type="number" step={step} value={draft} onChange={(event) => setDraft(event.target.value)} onBlur={commit} onKeyDown={(event) => { if (event.key === 'Enter') event.currentTarget.blur() }} /></label>
}

export function TerrainPanel({ controller }: { controller: EditorController }) {
  const terrain: TerrainSnapshot = controller.state.project.terrain
  const [seed, setSeed] = useState(terrain.seed)
  useEffect(() => setSeed(terrain.seed), [terrain.seed])

  if (!terrain.available || !terrain.entityId) {
    return <div className="panel-empty"><strong>No Terrain Volume</strong><span>Add a Terrain Volume component to an entity to enable generation.</span></div>
  }

  const setField = (fieldName: string, value: EngineValue) => controller.invoke('scene.setComponentField', {
    entityId: terrain.entityId,
    componentType: 'engine.terrain_volume',
    fieldName,
    value,
  })
  const number = (field: string) => (value: number) => { void setField(field, { Float32: value }) }
  const integer = (field: string) => (value: number) => { void setField(field, { UInt: Math.max(0, Math.floor(value)) }) }
  const stats = terrain.runtime

  return <div className="terrain-panel">
    <section className="inspector-section">
      <h3>Seed Replay</h3>
      <label className="inspector-field"><span>Seed (u64)</span><input value={seed} onChange={(event) => setSeed(event.target.value)} /></label>
      <div className="panel-actions">
        <button type="button" onClick={() => void controller.invoke('terrain.replaySeed', { seed })}>Replay Seed</button>
        <button type="button" onClick={() => void controller.invoke('terrain.regenerate', {})}>Regenerate</button>
        <button type="button" disabled={stats.failed === 0} onClick={() => void controller.invoke('terrain.retryFailed', {})}>Retry Failed</button>
      </div>
    </section>
    <section className="inspector-section terrain-runtime-grid">
      <h3>Runtime</h3>
      <span>Resident <strong>{stats.resident}</strong></span><span>Generating <strong>{stats.generating}</strong></span>
      <span>Queued <strong>{stats.queued}</strong></span><span>Ready <strong>{stats.readyToCommit}</strong></span>
      <span>Failed <strong>{stats.failed}</strong></span><span>Cache <strong>{formatBytes(stats.residentBytes)}</strong></span>
      <span>Last commit <strong>{formatBytes(stats.lastTickCommittedBytes)}</strong></span><span>Generation <strong>{(stats.lastGenerationMicros / 1000).toFixed(2)} ms</strong></span>
      <span>Stale dropped <strong>{stats.staleResultsDiscarded}</strong></span><span>Evicted <strong>{stats.evicted}</strong></span>
      {terrain.lastError && <p className="field-error">{terrain.lastError}</p>}
    </section>
    <section className="inspector-section">
      <h3>Hot Parameters</h3>
      <NumericField label="Chunk Size" value={terrain.chunkSize} onCommit={number('chunk_size')} />
      <NumericField label="Resolution" value={terrain.baseResolution} step="1" onCommit={integer('base_resolution')} />
      <NumericField label="Height Scale" value={terrain.heightScale} onCommit={number('height_scale')} />
      <NumericField label="Frequency" value={terrain.frequency} onCommit={number('frequency')} />
      <NumericField label="Octaves" value={terrain.octaves} step="1" onCommit={integer('octaves')} />
      <NumericField label="Lacunarity" value={terrain.lacunarity} onCommit={number('lacunarity')} />
      <NumericField label="Gain" value={terrain.gain} onCommit={number('gain')} />
      <NumericField label="Warp Amplitude" value={terrain.domainWarpAmplitude} onCommit={number('domain_warp_amplitude')} />
      <NumericField label="Warp Frequency" value={terrain.domainWarpFrequency} onCommit={number('domain_warp_frequency')} />
      <NumericField label="Skirt Depth" value={terrain.skirtDepth} onCommit={number('skirt_depth')} />
      <NumericField label="LOD Hysteresis" value={terrain.lodHysteresis} onCommit={number('lod_hysteresis')} />
      <label className="inspector-field"><span>Collision</span><input type="checkbox" checked={terrain.collisionEnabled} onChange={(event) => void setField('collision_enabled', { Bool: event.target.checked })} /></label>
      <p className="panel-hint">LOD distances remain editable in the Inspector list field. Every accepted change invalidates stale in-flight chunks.</p>
    </section>
  </div>
}
