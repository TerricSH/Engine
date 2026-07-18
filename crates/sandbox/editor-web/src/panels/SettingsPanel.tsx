import { useEffect, useRef, useState } from 'react'
import type {
  InputActionMapSnapshot,
  InputActionSnapshot,
  InputBindingSnapshot,
  InputModifierSnapshot,
  InputValueSnapshot,
  SceneSettingsSnapshot,
} from '../bridge/protocol'
import type { EditorController } from '../state/useEditorState'
import { Icon } from '../components/Icon'

type SettingsCategory = 'player' | 'input'
type InputValueType = InputActionSnapshot['value_type']
type BindingSource = 'KeyboardMouse' | 'GamepadButton' | 'GamepadAxis'
type ModifierKind = 'None' | 'Invert' | 'Deadzone' | 'Scale'

const KEY_CODES = [
  'W', 'A', 'S', 'D', 'Q', 'E', 'R', 'T', 'Y', 'U', 'I', 'O', 'P',
  'F', 'G', 'H', 'J', 'K', 'L', 'Z', 'X', 'C', 'V', 'B', 'N', 'M',
  'Digit0', 'Digit1', 'Digit2', 'Digit3', 'Digit4', 'Digit5', 'Digit6',
  'Digit7', 'Digit8', 'Digit9', 'Space', 'Enter', 'Escape', 'Tab',
  'Backspace', 'Delete', 'Up', 'Down', 'Left', 'Right', 'ShiftLeft',
  'ShiftRight', 'ControlLeft', 'ControlRight', 'AltLeft', 'AltRight',
  'MouseLeft', 'MouseRight', 'MouseMiddle',
] as const

const GAMEPAD_BUTTONS = [
  'A', 'B', 'X', 'Y', 'LB', 'RB', 'LT', 'RT', 'Start', 'Back',
  'DPadUp', 'DPadDown', 'DPadLeft', 'DPadRight',
] as const

const GAMEPAD_AXES = ['LeftX', 'LeftY', 'RightX', 'RightY', 'LT', 'RT'] as const

function currentValue(valueType: InputValueType): InputValueSnapshot {
  switch (valueType) {
    case 'Digital': return { Bool: false }
    case 'Analog1D': return { Float: 0 }
    case 'Analog2D': return { Vec2: [0, 0] }
  }
}

function makeBinding(action: string, source: BindingSource): InputBindingSnapshot {
  switch (source) {
    case 'KeyboardMouse':
      return { device: 'KeyboardMouse', action, keys: ['Space'], gamepad_button: null, gamepad_axis: null, modifier: 'None' }
    case 'GamepadButton':
      return { device: 'Gamepad', action, keys: [], gamepad_button: 'A', gamepad_axis: null, modifier: 'None' }
    case 'GamepadAxis':
      return { device: 'Gamepad', action, keys: [], gamepad_button: null, gamepad_axis: 'LeftX', modifier: 'None' }
  }
}

function defaultBindings(action: string, valueType: InputValueType): InputBindingSnapshot[] {
  if (valueType === 'Analog1D') return [makeBinding(action, 'GamepadAxis')]
  if (valueType === 'Analog2D') {
    return [
      makeBinding(action, 'GamepadAxis'),
      { ...makeBinding(action, 'GamepadAxis'), gamepad_axis: 'LeftY' },
    ]
  }
  return [makeBinding(action, 'KeyboardMouse')]
}

function createAction(name: string, valueType: InputValueType = 'Digital'): InputActionSnapshot {
  return { name, value_type: valueType, current_value: currentValue(valueType), bindings: defaultBindings(name, valueType) }
}

function uniqueActionName(map: InputActionMapSnapshot): string {
  const names = new Set(map.actions.map((action) => action.name))
  let candidate = 'new_action'
  let suffix = 2
  while (names.has(candidate)) candidate = `new_action_${suffix++}`
  return candidate
}

function sourceOf(binding: InputBindingSnapshot): BindingSource {
  if (binding.keys.length > 0) return 'KeyboardMouse'
  if (binding.gamepad_button !== null) return 'GamepadButton'
  return 'GamepadAxis'
}

function modifierKind(modifier: InputModifierSnapshot): ModifierKind {
  if (typeof modifier === 'string') return modifier
  return 'Deadzone' in modifier ? 'Deadzone' : 'Scale'
}

function modifierNumber(modifier: InputModifierSnapshot): number {
  if (typeof modifier === 'string') return modifier === 'Invert' ? -1 : 0
  return 'Deadzone' in modifier ? modifier.Deadzone : modifier.Scale
}

function validateInputMap(map: InputActionMapSnapshot): string[] {
  const errors: string[] = []
  if (!map.name.trim()) errors.push('Map name cannot be empty.')
  if (!map.context.trim()) errors.push('Context cannot be empty.')
  const names = new Set<string>()
  map.actions.forEach((action, actionIndex) => {
    const label = action.name.trim() || `Action ${actionIndex + 1}`
    if (!action.name.trim()) errors.push(`Action ${actionIndex + 1} needs a name.`)
    else if (names.has(action.name)) errors.push(`Action name “${action.name}” is duplicated.`)
    names.add(action.name)
    if (action.bindings.length === 0) errors.push(`${label} needs at least one binding.`)
    if (action.value_type === 'Digital' && !('Bool' in action.current_value)) errors.push(`${label} has an invalid current value.`)
    if (action.value_type === 'Analog1D' && !('Float' in action.current_value)) errors.push(`${label} has an invalid current value.`)
    if (action.value_type === 'Analog2D' && !('Vec2' in action.current_value)) errors.push(`${label} has an invalid current value.`)
    action.bindings.forEach((binding, bindingIndex) => {
      const bindingLabel = `${label}, binding ${bindingIndex + 1}`
      if (binding.action !== action.name) errors.push(`${bindingLabel} does not target its owning action.`)
      const sourceCount = Number(binding.keys.length > 0) + Number(binding.gamepad_button !== null) + Number(binding.gamepad_axis !== null)
      if (sourceCount !== 1) errors.push(`${bindingLabel} must use exactly one input source.`)
      if (binding.keys.length > 0 && binding.device !== 'KeyboardMouse') errors.push(`${bindingLabel} has an invalid keyboard device.`)
      if ((binding.gamepad_button !== null || binding.gamepad_axis !== null) && binding.device !== 'Gamepad') errors.push(`${bindingLabel} has an invalid gamepad device.`)
      if (action.value_type === 'Analog2D' && binding.keys.length > 0) errors.push(`${label} cannot use keyboard bindings for Analog2D.`)
      if (new Set(binding.keys).size !== binding.keys.length) errors.push(`${bindingLabel} contains duplicate keys.`)
      if (typeof binding.modifier !== 'string' && 'Deadzone' in binding.modifier
        && (!Number.isFinite(binding.modifier.Deadzone) || binding.modifier.Deadzone < 0 || binding.modifier.Deadzone > 1)) {
        errors.push(`${bindingLabel} deadzone must be between 0 and 1.`)
      }
      if (typeof binding.modifier !== 'string' && 'Scale' in binding.modifier && !Number.isFinite(binding.modifier.Scale)) {
        errors.push(`${bindingLabel} scale must be finite.`)
      }
    })
  })
  return errors
}

function mapFingerprint(map: InputActionMapSnapshot): string { return JSON.stringify(map) }

interface InputMapEditorProps { controller: EditorController; sourceMap: InputActionMapSnapshot }

function InputMapEditor({ controller, sourceMap }: InputMapEditorProps) {
  const [map, setMap] = useState<InputActionMapSnapshot>(sourceMap)
  const [status, setStatus] = useState('')
  const [confirmDelete, setConfirmDelete] = useState(false)
  const mapRef = useRef(map)
  const dirtyRef = useRef(false)
  const submitGeneration = useRef(0)

  useEffect(() => {
    const incoming = mapFingerprint(sourceMap)
    const local = mapFingerprint(mapRef.current)
    if (incoming === local) {
      dirtyRef.current = false
    } else if (!dirtyRef.current) {
      mapRef.current = sourceMap
      setMap(sourceMap)
      setStatus('')
    }
  }, [sourceMap])

  const commit = (next: InputActionMapSnapshot, message = 'Applying changes…') => {
    mapRef.current = next
    setMap(next)
    dirtyRef.current = true
    const errors = validateInputMap(next)
    if (errors.length > 0) {
      setStatus('Resolve validation errors before the map can be applied.')
      return
    }
    const generation = ++submitGeneration.current
    setStatus(message)
    void controller.invoke('settings.replaceInputMap', { map: next }).then((result) => {
      if (generation !== submitGeneration.current) return
      setStatus(result?.accepted ? 'Changes applied to the running project.' : 'The engine rejected this change.')
    })
  }

  const updateAction = (index: number, update: (action: InputActionSnapshot) => InputActionSnapshot) => {
    commit({ ...map, actions: map.actions.map((action, actionIndex) => actionIndex === index ? update(action) : action) })
  }

  const updateBinding = (actionIndex: number, bindingIndex: number, update: (binding: InputBindingSnapshot) => InputBindingSnapshot) => {
    updateAction(actionIndex, (action) => ({
      ...action,
      bindings: action.bindings.map((binding, index) => index === bindingIndex ? update(binding) : binding),
    }))
  }

  const errors = validateInputMap(map)

  const save = async () => {
    if (errors.length > 0) {
      setStatus('Resolve validation errors before saving.')
      return
    }
    setStatus('Applying and saving InputActions-v0…')
    const replaced = await controller.invoke('settings.replaceInputMap', { map })
    if (!replaced?.accepted) {
      setStatus('The engine rejected the map; it was not saved.')
      return
    }
    const saved = await controller.invoke('settings.saveInputMap', {})
    setStatus(saved?.accepted ? 'Input Actions saved to the project.' : 'Input Actions could not be saved.')
  }

  const resetMap = () => {
    const next = { name: 'player', context: 'gameplay', actions: [] }
    setConfirmDelete(false)
    commit(next, 'The empty “player” map is now active in the running project.')
  }

  return <div className="input-map-editor">
    <div className="input-map-header">
      <div className="input-map-active"><span>Project input map</span><strong>{map.name || 'Unnamed map'}</strong></div>
      <div className="input-map-header-actions">
        <button className="danger" type="button" onClick={() => setConfirmDelete(true)}>Reset Map…</button>
      </div>
    </div>

    {confirmDelete && <div className="input-map-confirm" role="alertdialog" aria-label="Reset input map">
      <div><strong>Reset “{map.name || 'Unnamed map'}”?</strong><p>An empty “player” map will replace the current runtime map. Save when you want to persist the replacement.</p></div>
      <button type="button" onClick={() => setConfirmDelete(false)}>Cancel</button>
      <button className="danger" type="button" onClick={resetMap}>Reset Map</button>
    </div>}

    <section className="input-map-properties">
      <label><span>Map name</span><input value={map.name} onChange={(event) => commit({ ...map, name: event.target.value })} /></label>
      <label><span>Context</span><input value={map.context} onChange={(event) => commit({ ...map, context: event.target.value })} /></label>
    </section>

    <div className="input-actions-heading">
      <div><h3>Actions</h3><small>{map.actions.length} configured</small></div>
      <button type="button" onClick={() => {
        const name = uniqueActionName(map)
        commit({ ...map, actions: [...map.actions, createAction(name)] })
      }}>+ Add Action</button>
    </div>

    <div className="input-actions-list">
      {map.actions.map((action, actionIndex) => <InputActionEditor
        key={actionIndex}
        action={action}
        actionIndex={actionIndex}
        onUpdate={(update) => updateAction(actionIndex, update)}
        onUpdateBinding={(bindingIndex, update) => updateBinding(actionIndex, bindingIndex, update)}
        onDelete={() => commit({ ...map, actions: map.actions.filter((_, index) => index !== actionIndex) })}
      />)}
      {map.actions.length === 0 && <div className="input-empty-state"><strong>No actions in this map</strong><span>Add an action to define keyboard, mouse, or gamepad controls.</span></div>}
    </div>

    {errors.length > 0 && <div className="input-validation" role="alert"><strong>Input map validation</strong><ul>{errors.map((error, index) => <li key={`${index}-${error}`}>{error}</li>)}</ul></div>}
    <div className="input-map-footer">
      <span className={errors.length > 0 ? 'invalid' : ''}>{status || (errors.length > 0 ? 'Changes are local until all validation errors are resolved.' : 'All changes are applied immediately; Save writes InputActions-v0 to disk.')}</span>
      <button className="primary" type="button" disabled={errors.length > 0} onClick={() => void save()}>Save Input Actions</button>
    </div>
  </div>
}

interface InputActionEditorProps {
  action: InputActionSnapshot
  actionIndex: number
  onUpdate(update: (action: InputActionSnapshot) => InputActionSnapshot): void
  onUpdateBinding(bindingIndex: number, update: (binding: InputBindingSnapshot) => InputBindingSnapshot): void
  onDelete(): void
}

function InputActionEditor({ action, actionIndex, onUpdate, onUpdateBinding, onDelete }: InputActionEditorProps) {
  const changeValueType = (valueType: InputValueType) => {
    onUpdate((current) => {
      let bindings = current.bindings
      if (valueType === 'Analog2D') bindings = bindings.filter((binding) => binding.keys.length === 0)
      if (bindings.length === 0) bindings = defaultBindings(current.name, valueType)
      return { ...current, value_type: valueType, current_value: currentValue(valueType), bindings }
    })
  }

  const addBinding = (source: BindingSource) => {
    onUpdate((current) => ({ ...current, bindings: [...current.bindings, makeBinding(current.name, source)] }))
  }

  return <article className="input-action-card">
    <header>
      <span className="input-action-index">{actionIndex + 1}</span>
      <input aria-label={`Action ${actionIndex + 1} name`} value={action.name} onChange={(event) => {
        const name = event.target.value
        onUpdate((current) => ({ ...current, name, bindings: current.bindings.map((binding) => ({ ...binding, action: name })) }))
      }} />
      <select aria-label={`${action.name} value type`} value={action.value_type} onChange={(event) => changeValueType(event.target.value as InputValueType)}>
        <option value="Digital">Digital</option><option value="Analog1D">Analog 1D</option><option value="Analog2D">Analog 2D</option>
      </select>
      <button className="danger icon-action" type="button" title={`Delete ${action.name || 'action'}`} onClick={onDelete}>×</button>
    </header>
    <div className="input-bindings">
      {action.bindings.map((binding, bindingIndex) => <BindingEditor
        key={bindingIndex}
        action={action}
        binding={binding}
        bindingIndex={bindingIndex}
        canDelete={action.bindings.length > 1}
        onUpdate={(update) => onUpdateBinding(bindingIndex, update)}
        onDelete={() => onUpdate((current) => ({ ...current, bindings: current.bindings.filter((_, index) => index !== bindingIndex) }))}
      />)}
    </div>
    <footer className="input-binding-add">
      {action.value_type !== 'Analog2D' && <button type="button" onClick={() => addBinding('KeyboardMouse')}>+ Keyboard / Mouse</button>}
      <button type="button" onClick={() => addBinding('GamepadButton')}>+ Gamepad Button</button>
      <button type="button" onClick={() => addBinding('GamepadAxis')}>+ Gamepad Axis</button>
    </footer>
  </article>
}

interface BindingEditorProps {
  action: InputActionSnapshot
  binding: InputBindingSnapshot
  bindingIndex: number
  canDelete: boolean
  onUpdate(update: (binding: InputBindingSnapshot) => InputBindingSnapshot): void
  onDelete(): void
}

function BindingEditor({ action, binding, bindingIndex, canDelete, onUpdate, onDelete }: BindingEditorProps) {
  const source = sourceOf(binding)
  const kind = modifierKind(binding.modifier)
  const changeSource = (nextSource: BindingSource) => {
    if (action.value_type === 'Analog2D' && nextSource === 'KeyboardMouse') return
    onUpdate(() => makeBinding(action.name, nextSource))
  }
  const changeModifier = (nextKind: ModifierKind) => {
    const modifier: InputModifierSnapshot = nextKind === 'Deadzone'
      ? { Deadzone: 0.15 }
      : nextKind === 'Scale'
        ? { Scale: 1 }
        : nextKind
    onUpdate((current) => ({ ...current, modifier }))
  }
  return <div className="input-binding-row">
    <span className="binding-order">{bindingIndex + 1}</span>
    <label><span>Device / source</span><select value={source} onChange={(event) => changeSource(event.target.value as BindingSource)}>
      {action.value_type !== 'Analog2D' && <option value="KeyboardMouse">Keyboard / Mouse</option>}
      <option value="GamepadButton">Gamepad Button</option><option value="GamepadAxis">Gamepad Axis</option>
    </select></label>
    <label><span>Action</span><input value={binding.action} readOnly title="Bindings always target their owning action" /></label>
    {source === 'KeyboardMouse' && <div className="binding-keys"><span>Keys / buttons</span><div className="binding-key-chips">
      {binding.keys.map((key) => <button type="button" key={key} title={`Remove ${key}`} disabled={binding.keys.length === 1} onClick={() => onUpdate((current) => ({ ...current, keys: current.keys.filter((candidate) => candidate !== key) }))}>{key}<i>×</i></button>)}
      <select aria-label="Add key or mouse button" value="" onChange={(event) => {
        const key = event.target.value
        if (key && !binding.keys.includes(key)) onUpdate((current) => ({ ...current, keys: [...current.keys, key] }))
      }}><option value="">+ Add key</option>{KEY_CODES.filter((key) => !binding.keys.includes(key)).map((key) => <option key={key} value={key}>{key}</option>)}</select>
    </div></div>}
    {source === 'GamepadButton' && <label><span>Button</span><select value={binding.gamepad_button ?? 'A'} onChange={(event) => onUpdate((current) => ({ ...current, gamepad_button: event.target.value }))}>{GAMEPAD_BUTTONS.map((button) => <option key={button}>{button}</option>)}</select></label>}
    {source === 'GamepadAxis' && <label><span>Axis</span><select value={binding.gamepad_axis ?? 'LeftX'} onChange={(event) => onUpdate((current) => ({ ...current, gamepad_axis: event.target.value }))}>{GAMEPAD_AXES.map((axis) => <option key={axis}>{axis}</option>)}</select></label>}
    <label><span>Modifier</span><select value={kind} onChange={(event) => changeModifier(event.target.value as ModifierKind)}><option value="None">None</option><option value="Invert">Invert</option><option value="Deadzone">Deadzone</option><option value="Scale">Scale</option></select></label>
    {(kind === 'Deadzone' || kind === 'Scale') && <label><span>{kind === 'Deadzone' ? 'Threshold' : 'Factor'}</span><input type="number" step="any" min={kind === 'Deadzone' ? 0 : undefined} max={kind === 'Deadzone' ? 1 : undefined} value={modifierNumber(binding.modifier)} onChange={(event) => {
      const value = Number(event.target.value)
      onUpdate((current) => ({ ...current, modifier: kind === 'Deadzone' ? { Deadzone: value } : { Scale: value } }))
    }} /></label>}
    <button className="danger icon-action binding-delete" type="button" title={canDelete ? 'Delete binding' : 'Every action requires at least one binding'} disabled={!canDelete} onClick={onDelete}>×</button>
  </div>
}

const STANDARD_RENDER_PASSES = ['DirectionalShadow', 'OpaquePbrForward', 'ToneMap', 'Present'] as const

function normalizeRenderPasses(
  passes: SceneSettingsSnapshot['pass_graph_config']['passes'],
  outputMode: SceneSettingsSnapshot['pass_graph_config']['output_mode'],
  shadowOverride?: boolean,
) {
  const enabled = new Map(passes.map((pass) => [pass.kind, pass.enabled]))
  const custom = passes.filter((pass) => !STANDARD_RENDER_PASSES.includes(pass.kind as typeof STANDARD_RENDER_PASSES[number]))
  return [
    { kind: 'DirectionalShadow', enabled: shadowOverride ?? enabled.get('DirectionalShadow') ?? true },
    { kind: 'OpaquePbrForward', enabled: true },
    ...custom,
    { kind: 'ToneMap', enabled: outputMode === 'HdrThenToneMap' },
    { kind: 'Present', enabled: true },
  ]
}

function validateSceneSettings(scene: SceneSettingsSnapshot): string[] {
  const errors: string[] = []
  if (!scene.default_render_layer.trim()) errors.push('Default render layer cannot be empty.')
  if (!Number.isFinite(scene.fixed_timestep_seconds) || scene.fixed_timestep_seconds <= 0 || scene.fixed_timestep_seconds > 1) errors.push('Fixed timestep must be between 0 and 1 second.')
  if (scene.gravity?.some((value) => !Number.isFinite(value))) errors.push('Gravity values must be finite.')
  if (scene.ambient.some((value) => !Number.isFinite(value) || value < 0)) errors.push('Ambient values must be finite and non-negative.')
  if (scene.environment_map && !scene.environment_map.id.trim()) errors.push('Environment map must reference a valid texture.')
  if (scene.pass_graph_config.enabled && scene.pass_graph_config.output_mode === 'DirectToSwapchain' && scene.tone_mapping !== 'None') errors.push('Direct-to-swapchain output requires tone mapping None.')
  return errors
}

function SceneSettingsEditor({ controller, scene, setScene }: { controller: EditorController; scene: SceneSettingsSnapshot; setScene(scene: SceneSettingsSnapshot): void }) {
  const [status, setStatus] = useState('')
  const project = controller.state.project
  const editing = project.capabilities.editing && !project.capabilities.buildBusy
  const errors = validateSceneSettings(scene)
  const environmentTextures = project.assets.filter((asset) => asset.kind === 'texture')
  const assetKey = (asset: NonNullable<SceneSettingsSnapshot['environment_map']>) => JSON.stringify([asset.id, asset.logical_path])
  const currentEnvironment = scene.environment_map ? assetKey(scene.environment_map) : ''
  const updateVector = <K extends 'gravity' | 'ambient'>(key: K, index: number, value: number) => {
    const current = scene[key]
    if (!current) return
    const next = [...current]
    next[index] = value
    setScene({ ...scene, [key]: next })
  }
  const changeOutputMode = (outputMode: SceneSettingsSnapshot['pass_graph_config']['output_mode']) => setScene({
    ...scene,
    tone_mapping: outputMode === 'DirectToSwapchain' ? 'None' : scene.tone_mapping,
    pass_graph_config: {
      ...scene.pass_graph_config,
      output_mode: outputMode,
      passes: normalizeRenderPasses(scene.pass_graph_config.passes, outputMode),
    },
  })
  const apply = async () => {
    if (errors.length > 0) return
    setStatus('Applying scene settings…')
    const result = await controller.invoke('scene.applySettings', { settings: scene })
    setStatus(result?.accepted ? 'Scene settings applied.' : 'The engine rejected these scene settings.')
  }
  return <>
    <h2 className="settings-section-heading">Scene</h2>
    <label className="setting-row"><span><strong>Active camera</strong><small>Camera used by the game viewport</small></span><select value={scene.active_camera ?? ''} onChange={(event) => setScene({ ...scene, active_camera: event.target.value || null })}>
      <option value="">Automatic</option>
      {scene.active_camera && !project.settings.cameraEntities.some((camera) => camera.id === scene.active_camera) && <option value={scene.active_camera}>{scene.active_camera} (unavailable)</option>}
      {project.settings.cameraEntities.map((camera) => <option key={camera.id} value={camera.id}>{camera.name} · {camera.id}</option>)}
    </select></label>
    <label className="setting-row"><span><strong>Default render layer</strong></span><input value={scene.default_render_layer} onChange={(event) => setScene({ ...scene, default_render_layer: event.target.value })} /></label>
    <label className="setting-row"><span><strong>Fixed timestep</strong><small>Seconds per fixed update, up to 1</small></span><input type="number" min={0.0001} max={1} step="any" value={scene.fixed_timestep_seconds} onChange={(event) => setScene({ ...scene, fixed_timestep_seconds: Number(event.target.value) })} /></label>
    <div className="setting-row"><span><strong>Gravity</strong><small>Optional world-space acceleration</small></span><div className="setting-control-stack"><label className="setting-check"><input type="checkbox" checked={scene.gravity !== null} onChange={(event) => setScene({ ...scene, gravity: event.target.checked ? [0, -9.81, 0] : null })} />Enabled</label>{scene.gravity && <div className="setting-vector">{scene.gravity.map((value, index) => <label key={index}><small>{['X', 'Y', 'Z'][index]}</small><input aria-label={`Gravity ${['X', 'Y', 'Z'][index]}`} type="number" step="any" value={value} onChange={(event) => updateVector('gravity', index, Number(event.target.value))} /></label>)}</div>}</div></div>
    <div className="setting-row"><span><strong>Ambient color</strong><small>Linear RGBA values</small></span><div className="setting-vector">{scene.ambient.map((value, index) => <label key={index}><small>{['R', 'G', 'B', 'A'][index]}</small><input aria-label={`Ambient ${['R', 'G', 'B', 'A'][index]}`} type="number" min={0} step="any" value={value} onChange={(event) => updateVector('ambient', index, Number(event.target.value))} /></label>)}</div></div>
    <label className="setting-row"><span><strong>Environment map</strong><small>Optional texture asset</small></span><select value={currentEnvironment} onChange={(event) => { const asset = environmentTextures.find((candidate) => assetKey(candidate.assetId) === event.target.value); setScene({ ...scene, environment_map: asset?.assetId ?? null }) }}>
      <option value="">None</option>
      {scene.environment_map && !environmentTextures.some((asset) => assetKey(asset.assetId) === currentEnvironment) && <option value={currentEnvironment}>{scene.environment_map.id} (unavailable)</option>}
      {environmentTextures.map((asset) => <option key={assetKey(asset.assetId)} value={assetKey(asset.assetId)}>{asset.name}</option>)}
    </select></label>
    <label className="setting-row"><span><strong>Tone mapping</strong></span><select value={scene.tone_mapping} disabled={scene.pass_graph_config.output_mode === 'DirectToSwapchain'} onChange={(event) => setScene({ ...scene, tone_mapping: event.target.value as SceneSettingsSnapshot['tone_mapping'] })}><option value="Aces">ACES</option><option value="Reinhard">Reinhard</option><option value="None">None</option></select></label>
    <h2 className="settings-section-heading">Render Graph</h2>
    <label className="setting-row"><span><strong>Custom pass graph</strong><small>Use the scene pass configuration</small></span><span className="setting-check"><input type="checkbox" checked={scene.pass_graph_config.enabled} onChange={(event) => setScene({ ...scene, pass_graph_config: { ...scene.pass_graph_config, enabled: event.target.checked, passes: event.target.checked ? normalizeRenderPasses(scene.pass_graph_config.passes, scene.pass_graph_config.output_mode) : scene.pass_graph_config.passes } })} />Enabled</span></label>
    <label className="setting-row"><span><strong>Output mode</strong></span><select disabled={!scene.pass_graph_config.enabled} value={scene.pass_graph_config.output_mode} onChange={(event) => changeOutputMode(event.target.value as SceneSettingsSnapshot['pass_graph_config']['output_mode'])}><option value="HdrThenToneMap">HDR then tone map</option><option value="DirectToSwapchain">Direct to swapchain</option></select></label>
    {scene.pass_graph_config.enabled && <div className="render-pass-list">{scene.pass_graph_config.passes.map((pass, index) => {
      const required = pass.kind === 'OpaquePbrForward' || pass.kind === 'Present' || pass.kind === 'ToneMap'
      return <label key={`${pass.kind}-${index}`}><input type="checkbox" checked={pass.enabled} disabled={required} onChange={(event) => setScene({ ...scene, pass_graph_config: { ...scene.pass_graph_config, passes: pass.kind === 'DirectionalShadow' ? normalizeRenderPasses(scene.pass_graph_config.passes, scene.pass_graph_config.output_mode, event.target.checked) : scene.pass_graph_config.passes.map((candidate, candidateIndex) => candidateIndex === index ? { ...candidate, enabled: event.target.checked } : candidate) } })} /><span>{pass.kind}</span>{required && <small>Required</small>}</label>
    })}</div>}
    {errors.length > 0 && <div className="settings-validation" role="alert">{errors.map((error) => <span key={error}>{error}</span>)}</div>}
    <div className="settings-apply-row"><span>{status}</span><button type="button" disabled={!editing || errors.length > 0} onClick={() => void apply()}>Apply Scene Settings</button></div>
  </>
}

export function SettingsPanel({ controller }: { controller: EditorController }) {
  const settings = controller.state.project.settings
  const [category, setCategory] = useState<SettingsCategory>('player')
  const [title, setTitle] = useState(settings.windowTitle)
  const [width, setWidth] = useState(settings.windowWidth)
  const [height, setHeight] = useState(settings.windowHeight)
  const [scene, setScene] = useState<SceneSettingsSnapshot>(settings.sceneSettings)
  const sceneFingerprint = JSON.stringify(settings.sceneSettings)
  useEffect(() => { setTitle(settings.windowTitle); setWidth(settings.windowWidth); setHeight(settings.windowHeight); setScene(settings.sceneSettings) }, [settings.windowTitle, settings.windowWidth, settings.windowHeight, sceneFingerprint])
  return <div className="settings-panel">
    <nav className="settings-categories panel-scroll">
      <div className="settings-heading"><Icon name="settings" /><span>Project Settings</span></div>
      <button className={category === 'player' ? 'active' : ''} type="button" onClick={() => setCategory('player')}>Player</button>
      <button className={category === 'input' ? 'active' : ''} type="button" onClick={() => setCategory('input')}>Input</button>
    </nav>
    <div className="settings-page panel-scroll">
      {category === 'player' ? <>
        <h2>Player</h2>
        <label className="setting-row"><span><strong>Window title</strong></span><input value={title} onChange={(event) => setTitle(event.target.value)} /></label>
        <label className="setting-row"><span><strong>Window width</strong></span><input type="number" min={1} value={width} onChange={(event) => setWidth(Number(event.target.value))} /></label>
        <label className="setting-row"><span><strong>Window height</strong></span><input type="number" min={1} value={height} onChange={(event) => setHeight(Number(event.target.value))} /></label>
        <button className="primary" type="button" onClick={() => void controller.invoke('project.saveSettings', { title, width, height })}>Save Player Settings</button>
        <SceneSettingsEditor controller={controller} scene={scene} setScene={setScene} />
      </> : <>
        <div className="settings-page-title"><div><h2>Input Actions</h2><p>Author the project’s runtime input map. Valid changes are applied immediately.</p></div><span>InputActions-v0</span></div>
        <InputMapEditor controller={controller} sourceMap={settings.inputMap} />
      </>}
    </div>
  </div>
}
