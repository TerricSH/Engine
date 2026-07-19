import { useEffect, useMemo, useRef, useState } from 'react'
import { Icon } from './Icon'

const commands = [
  ['file.newScene', 'Create new scene', 'File'],
  ['file.openScene', 'Open project scene', 'File'],
  ['file.createProject', 'Create project in new window', 'File'],
  ['file.openProject', 'Open project in new window', 'File'],
  ['file.saveScene', 'Save current scene', 'File'],
  ['file.saveSceneAs', 'Save current scene as', 'File'],
  ['file.build', 'Open build settings', 'Build'],
  ['edit.undo', 'Undo last action', 'Edit'],
  ['edit.redo', 'Redo last action', 'Edit'],
  ['edit.cut', 'Cut selected GameObjects', 'Edit'],
  ['edit.copy', 'Copy selected GameObjects', 'Edit'],
  ['edit.paste', 'Paste GameObjects', 'Edit'],
  ['edit.duplicate', 'Duplicate selected GameObject', 'Edit'],
  ['edit.delete', 'Delete selected GameObject', 'Edit'],
  ['edit.projectSettings', 'Open project settings', 'Edit'],
  ['assets.import', 'Import and cook asset', 'Assets'],
  ['assets.createFolder', 'Create asset folder', 'Assets'],
  ['assets.createMaterial', 'Create material', 'Assets'],
  ['assets.createScript', 'Create C# script', 'Assets'],
  ['assets.createPrefab', 'Create prefab from selection', 'Assets'],
  ['assets.refresh', 'Refresh and recook assets', 'Assets'],
  ['gameObject.empty', 'Create empty GameObject', 'Scene'],
  ['gameObject.cube', 'Create cube', 'Scene'],
  ['gameObject.camera', 'Create camera', 'Scene'],
  ['gameObject.light', 'Create directional light', 'Scene'],
  ['gameObject.audioListener', 'Create audio listener', 'Scene'],
  ['component.add', 'Add component to selection', 'Inspector'],
  ['component.resetTransform', 'Reset selected transform', 'Inspector'],
  ['window.scene', 'Focus Scene view', 'Window'],
  ['window.game', 'Focus Game view', 'Window'],
  ['window.hierarchy', 'Show Hierarchy', 'Window'],
  ['window.inspector', 'Show Inspector', 'Window'],
  ['window.project', 'Show Project', 'Window'],
  ['window.console', 'Show Console', 'Window'],
  ['window.material', 'Show Material editor', 'Window'],
  ['window.animation', 'Show Animation', 'Window'],
  ['window.profiler', 'Show Profiler', 'Window'],
  ['window.terrain', 'Show Terrain Debugger', 'Window'],
  ['window.build', 'Show Build', 'Window'],
  ['window.resetLayout', 'Restore default layout', 'Window'],
  ['viewport.focusSelection', 'Frame selected object', 'Scene'],
] as const

interface CommandPaletteProps {
  open: boolean
  disabledCommands: ReadonlySet<string>
  onClose(): void
  onCommand(command: string): void
}

export function CommandPalette({ open, disabledCommands, onClose, onCommand }: CommandPaletteProps) {
  const [query, setQuery] = useState('')
  const [activeIndex, setActiveIndex] = useState(0)
  const inputRef = useRef<HTMLInputElement>(null)
  const filtered = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase()
    return commands.filter(([id, label, category]) => !disabledCommands.has(id) && `${label} ${category}`.toLocaleLowerCase().includes(normalized))
  }, [disabledCommands, query])

  useEffect(() => {
    if (!open) return
    setQuery('')
    setActiveIndex(0)
    window.requestAnimationFrame(() => inputRef.current?.focus())
  }, [open])

  if (!open) return null
  return (
    <div className="modal-backdrop" onPointerDown={onClose}>
      <div className="command-palette" role="dialog" aria-modal="true" aria-label="Command palette" onPointerDown={(event) => event.stopPropagation()}>
        <div className="command-input">
          <Icon name="search" />
          <input
            ref={inputRef}
            value={query}
            placeholder="Type a command…"
            onChange={(event) => { setQuery(event.target.value); setActiveIndex(0) }}
            onKeyDown={(event) => {
              if (event.key === 'Escape') onClose()
              if (event.key === 'ArrowDown') { event.preventDefault(); setActiveIndex((index) => Math.min(index + 1, filtered.length - 1)) }
              if (event.key === 'ArrowUp') { event.preventDefault(); setActiveIndex((index) => Math.max(index - 1, 0)) }
              if (event.key === 'Enter' && filtered[activeIndex]) {
                onCommand(filtered[activeIndex][0])
                onClose()
              }
            }}
          />
          <kbd>Esc</kbd>
        </div>
        <div className="command-results">
          {filtered.map(([id, label, category], index) => (
            <button
              className={index === activeIndex ? 'active' : ''}
              type="button"
              key={id}
              onPointerEnter={() => setActiveIndex(index)}
              onClick={() => { onCommand(id); onClose() }}
            >
              <span>{label}</span><small>{category}</small>
            </button>
          ))}
          {filtered.length === 0 && <div className="empty-search">No matching commands</div>}
        </div>
      </div>
    </div>
  )
}
