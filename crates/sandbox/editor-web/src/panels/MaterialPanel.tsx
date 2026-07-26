import { useEffect, useMemo, useState } from 'react'
import type { MaterialParameterSnapshot } from '../bridge/protocol'
import { Icon } from '../components/Icon'
import type { EditorController } from '../state/useEditorState'

type Rgba = [number, number, number, number]

function finiteNumber(value: unknown, fallback = 0): number {
  return typeof value === 'number' && Number.isFinite(value) ? value : fallback
}

function clampUnit(value: number): number {
  return Math.max(0, Math.min(1, value))
}

function rgbaValue(value: unknown): Rgba {
  if (!Array.isArray(value) || value.length !== 4) return [1, 1, 1, 1]
  return [
    clampUnit(finiteNumber(value[0], 1)),
    clampUnit(finiteNumber(value[1], 1)),
    clampUnit(finiteNumber(value[2], 1)),
    clampUnit(finiteNumber(value[3], 1)),
  ]
}

function toHex(color: Rgba): string {
  return `#${color.slice(0, 3).map((channel) => Math.round(clampUnit(channel) * 255).toString(16).padStart(2, '0')).join('')}`
}

function fromHex(value: string, alpha: number): Rgba {
  const hex = value.replace('#', '')
  if (!/^[0-9a-f]{6}$/i.test(hex)) return [1, 1, 1, alpha]
  return [
    Number.parseInt(hex.slice(0, 2), 16) / 255,
    Number.parseInt(hex.slice(2, 4), 16) / 255,
    Number.parseInt(hex.slice(4, 6), 16) / 255,
    alpha,
  ]
}

interface ParameterEditorProps {
  parameter: MaterialParameterSnapshot
  disabled: boolean
  textureAssets: { id: string; name: string }[]
  onCommit(name: string, value: unknown): Promise<void>
}

function FloatParameterEditor({ parameter, disabled, onCommit }: ParameterEditorProps) {
  const source = finiteNumber(parameter.value)
  const [draft, setDraft] = useState(String(source))

  useEffect(() => setDraft(String(source)), [source])

  const commit = async (raw: string) => {
    const parsed = Number(raw)
    if (!Number.isFinite(parsed)) {
      setDraft(String(source))
      return
    }
    const next = clampUnit(parsed)
    setDraft(String(next))
    if (next !== source) await onCommit(parameter.name, next)
  }

  return (
    <div className="material-parameter-row">
      <label htmlFor={`material-${parameter.name}`}>{parameter.name}</label>
      <div className="material-float-editor">
        <input
          aria-label={`${parameter.name} slider`}
          disabled={disabled}
          max={1}
          min={0}
          onChange={(event) => setDraft(event.target.value)}
          onKeyUp={(event) => {
            if (event.key === 'Enter' || event.key.startsWith('Arrow') || event.key === 'Home' || event.key === 'End' || event.key.startsWith('Page')) {
              void commit(event.currentTarget.value)
            }
          }}
          onPointerUp={(event) => void commit(event.currentTarget.value)}
          step={0.01}
          type="range"
          value={Number.isFinite(Number(draft)) ? clampUnit(Number(draft)) : source}
        />
        <input
          id={`material-${parameter.name}`}
          aria-label={`${parameter.name} value`}
          disabled={disabled}
          max={1}
          min={0}
          onBlur={(event) => void commit(event.currentTarget.value)}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
            if (event.key === 'Escape') { setDraft(String(source)); event.currentTarget.blur() }
          }}
          step={0.01}
          type="number"
          value={draft}
        />
      </div>
    </div>
  )
}

function ColorParameterEditor({ parameter, disabled, onCommit }: ParameterEditorProps) {
  const sourceFingerprint = JSON.stringify(rgbaValue(parameter.value))
  const source = useMemo(() => rgbaValue(parameter.value), [sourceFingerprint])
  const [draft, setDraft] = useState<Rgba>(source)

  useEffect(() => setDraft(source), [source])

  const commit = async (next: Rgba) => {
    const normalized = next.map((channel) => clampUnit(finiteNumber(channel))) as Rgba
    setDraft(normalized)
    if (normalized.some((channel, index) => channel !== source[index])) {
      await onCommit(parameter.name, normalized)
    }
  }

  const updateChannel = (index: number, raw: string) => {
    const parsed = Number(raw)
    if (!Number.isFinite(parsed)) return
    setDraft((current) => current.map((channel, channelIndex) => channelIndex === index ? parsed : channel) as Rgba)
  }

  return (
    <div className="material-parameter-row material-color-row">
      <label htmlFor={`material-${parameter.name}`}>{parameter.name}</label>
      <div className="material-color-editor">
        <input
          id={`material-${parameter.name}`}
          aria-label={`${parameter.name} color`}
          disabled={disabled}
          onBlur={() => void commit(draft)}
          onChange={(event) => setDraft(fromHex(event.target.value, draft[3]))}
          type="color"
          value={toHex(draft)}
        />
        <div className="material-color-channels">
          {(['R', 'G', 'B', 'A'] as const).map((channel, index) => (
            <label key={channel}>
              <span>{channel}</span>
              <input
                aria-label={`${parameter.name} ${channel}`}
                disabled={disabled}
                max={1}
                min={0}
                onBlur={() => void commit(draft)}
                onChange={(event) => updateChannel(index, event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') event.currentTarget.blur()
                  if (event.key === 'Escape') { setDraft(source); event.currentTarget.blur() }
                }}
                step={0.01}
                type="number"
                value={draft[index]}
              />
            </label>
          ))}
        </div>
      </div>
    </div>
  )
}

function TextureParameterEditor({ parameter, disabled, textureAssets, onCommit }: ParameterEditorProps) {
  const source = typeof parameter.value === 'string' ? parameter.value : ''
  const [draft, setDraft] = useState(source)
  const listId = `material-textures-${parameter.name.replaceAll(' ', '-')}`

  useEffect(() => setDraft(source), [source])

  const commit = async (raw: string) => {
    const next = raw.trim()
    setDraft(next)
    if (next !== source) await onCommit(parameter.name, next || null)
  }

  return (
    <div className="material-parameter-row">
      <label htmlFor={`material-${parameter.name}`}>{parameter.name}</label>
      <div className="material-texture-editor">
        <span className="material-texture-preview"><Icon name="scene" /></span>
        <input
          id={`material-${parameter.name}`}
          aria-label={`${parameter.name} texture asset`}
          disabled={disabled}
          list={listId}
          onBlur={(event) => void commit(event.currentTarget.value)}
          onChange={(event) => setDraft(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') event.currentTarget.blur()
            if (event.key === 'Escape') { setDraft(source); event.currentTarget.blur() }
          }}
          placeholder="None (texture asset id)"
          value={draft}
        />
        <datalist id={listId}>{textureAssets.map((asset) => <option key={asset.id} value={asset.id}>{asset.name}</option>)}</datalist>
        <button
          aria-label={`Clear ${parameter.name}`}
          disabled={disabled || !draft}
          onClick={() => { setDraft(''); void commit('') }}
          title="Clear texture binding"
          type="button"
        ><Icon name="close" /></button>
      </div>
    </div>
  )
}

function ChoiceParameterEditor({ parameter, disabled, onCommit }: ParameterEditorProps) {
  const source = typeof parameter.value === 'string' ? parameter.value : ''
  const options = parameter.options ?? []
  return (
    <div className="material-parameter-row">
      <label htmlFor={`material-${parameter.name}`}>{parameter.name}</label>
      <select
        id={`material-${parameter.name}`}
        aria-label={`${parameter.name} choice`}
        disabled={disabled}
        onChange={(event) => void onCommit(parameter.name, event.currentTarget.value)}
        value={source}
      >
        {options.map((option) => <option key={option} value={option}>{option}</option>)}
      </select>
    </div>
  )
}

function BoolParameterEditor({ parameter, disabled, onCommit }: ParameterEditorProps) {
  const source = parameter.value === true
  return (
    <div className="material-parameter-row">
      <label htmlFor={`material-${parameter.name}`}>{parameter.name}</label>
      <input
        id={`material-${parameter.name}`}
        aria-label={`${parameter.name} toggle`}
        checked={source}
        disabled={disabled}
        onChange={(event) => void onCommit(parameter.name, event.currentTarget.checked)}
        type="checkbox"
      />
    </div>
  )
}

function MaterialParameterEditor(props: ParameterEditorProps) {
  switch (props.parameter.kind) {
    case 'float': return <FloatParameterEditor {...props} />
    case 'color': return <ColorParameterEditor {...props} />
    case 'texture': return <TextureParameterEditor {...props} />
    case 'choice': return <ChoiceParameterEditor {...props} />
    case 'bool': return <BoolParameterEditor {...props} />
  }
}

export function MaterialPanel({ controller }: { controller: EditorController }) {
  const { project } = controller.state
  const { material, capabilities } = project
  const [busyParameter, setBusyParameter] = useState<string>()
  const [saving, setSaving] = useState(false)
  const [assigning, setAssigning] = useState(false)
  const [dirty, setDirty] = useState(false)

  const materialAssets = useMemo(() => project.assets
    .filter((asset) => asset.kind === 'material')
    .sort((left, right) => left.name.localeCompare(right.name)), [project.assets])
  const textureAssets = useMemo(() => project.assets
    .filter((asset) => asset.kind === 'texture')
    .map((asset) => ({ id: asset.id, name: asset.name }))
    .sort((left, right) => left.name.localeCompare(right.name)), [project.assets])
  const selectedMaterial = material.selectedMaterial ?? ''
  const selectedInCatalog = materialAssets.some((asset) => asset.id === selectedMaterial)
  const canEditParameters = Boolean(selectedMaterial) && material.writable && capabilities.editing && !capabilities.buildBusy
  const canAssign = Boolean(selectedMaterial) && capabilities.editing && !capabilities.buildBusy && capabilities.hasSelection
  const shaderName = material.parameters.length > 0 ? 'Standard PBR (MaterialSource v0)' : 'Unavailable'

  useEffect(() => setDirty(false), [selectedMaterial])
  useEffect(() => {
    if (!material.saveStatus) return
    setDirty(material.saveStatus.toLocaleLowerCase().includes('fail'))
  }, [material.saveStatus])

  const openMaterial = (assetId: string) => {
    if (!assetId) return
    const asset = materialAssets.find((candidate) => candidate.id === assetId)
    if (asset) controller.selectAsset(asset.assetId)
    void controller.invoke('material.open', { assetId })
  }

  const commitParameter = async (name: string, value: unknown) => {
    if (!canEditParameters || busyParameter) return
    setBusyParameter(name)
    const result = await controller.invoke('material.setParameter', { name, value })
    if (result?.accepted) setDirty(true)
    setBusyParameter(undefined)
  }

  const save = async () => {
    if (!canEditParameters || saving) return
    setSaving(true)
    const result = await controller.invoke('material.save', {})
    if (result?.accepted) {
      const snapshot = await controller.invoke('editor.getSnapshot', {})
      setDirty(Boolean(snapshot?.material.saveStatus?.toLocaleLowerCase().includes('fail')))
    }
    setSaving(false)
  }

  const assign = async () => {
    if (!canAssign || assigning) return
    setAssigning(true)
    await controller.invoke('material.assign', {})
    setAssigning(false)
  }

  const editDisabledReason = !selectedMaterial
    ? 'Select a project material first'
    : !capabilities.editing
      ? 'Material authoring is available only in Edit mode'
      : capabilities.buildBusy
        ? 'Wait for the active project operation to finish'
      : material.readOnlyReason

  const assignDisabledReason = !selectedMaterial
    ? 'Select a material first'
    : !capabilities.editing
      ? 'Material assignment is available only in Edit mode'
      : capabilities.buildBusy
        ? 'Wait for the active project operation to finish'
      : !capabilities.hasSelection
        ? 'Select an entity with a Renderable component'
        : undefined

  return (
    <div className="material-panel panel-column">
      <div className="material-toolbar">
        <Icon name="sphere" />
        <select
          aria-label="Open material"
          disabled={materialAssets.length === 0}
          onChange={(event) => openMaterial(event.target.value)}
          title={materialAssets.length === 0 ? 'No material assets are present in this project' : 'Open a project material'}
          value={selectedMaterial}
        >
          <option value="">Select Material...</option>
          {selectedMaterial && !selectedInCatalog && <option value={selectedMaterial}>{selectedMaterial} (external)</option>}
          {materialAssets.map((asset) => <option key={asset.id} value={asset.id}>{asset.name} — {asset.id}</option>)}
        </select>
        <span className={`material-access ${material.writable ? 'writable' : 'readonly'}`} title={material.readOnlyReason}>
          <Icon name={material.writable ? 'eye' : 'lock'} />
          {material.writable ? 'Writable' : 'Read only'}
        </span>
        <div className="material-toolbar-spacer" />
        <button
          className="material-action-button"
          disabled={!canAssign || assigning}
          onClick={() => void assign()}
          title={assignDisabledReason ?? 'Assign this material to the selected entity'}
          type="button"
        ><Icon name="link" />{assigning ? 'Assigning...' : 'Assign'}</button>
        <button
          className="material-action-button primary"
          disabled={!canEditParameters || saving}
          onClick={() => void save()}
          title={editDisabledReason ?? 'Persist parameters to the project material source'}
          type="button"
        >{saving ? 'Saving...' : 'Save'}</button>
      </div>

      {!selectedMaterial ? (
        <div className="panel-empty">
          <Icon name="sphere" />
          <span>Select a material above or double-click one in the Project panel.</span>
        </div>
      ) : (
        <div className="material-content panel-scroll">
          <section className="material-summary">
            <div className="material-identity">
              <strong>{selectedMaterial}</strong>
              <dl>
                <div><dt>Shader</dt><dd>{shaderName}</dd></div>
                <div><dt>Source</dt><dd>{material.writable ? 'Project material' : 'Runtime / external'}</dd></div>
                <div><dt>Parameters</dt><dd>{material.parameters.length}</dd></div>
              </dl>
            </div>
          </section>

          {material.readOnlyReason && <div className="material-notice warning"><Icon name="lock" /><span>{material.readOnlyReason}</span></div>}
          {dirty && <div className="material-notice modified" role="status"><Icon name="warning" /><span>Material parameters have unsaved changes.</span></div>}
          {material.saveStatus && !dirty && <div className={`material-notice ${material.saveStatus.toLocaleLowerCase().includes('fail') ? 'error' : 'status'}`} role="status"><Icon name={material.saveStatus.toLocaleLowerCase().includes('fail') ? 'error' : 'info'} /><span>{material.saveStatus}</span></div>}

          <section className="material-parameters">
            <header><strong>Surface Inputs</strong><span>{canEditParameters ? 'Project source is editable' : editDisabledReason}</span></header>
            {material.parameters.map((parameter) => (
              <MaterialParameterEditor
                disabled={!canEditParameters || Boolean(busyParameter)}
                key={`${parameter.kind}:${parameter.name}`}
                onCommit={commitParameter}
                parameter={parameter}
                textureAssets={textureAssets}
              />
            ))}
            {material.parameters.length === 0 && <div className="material-parameters-empty">No runtime material parameters are available. Reimport the asset and resolve its diagnostics.</div>}
          </section>
        </div>
      )}
    </div>
  )
}
