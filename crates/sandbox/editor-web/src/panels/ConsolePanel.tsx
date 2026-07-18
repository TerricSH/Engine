import { useMemo, useState } from 'react'
import type { ConsoleEntry, ConsoleLevel } from '../bridge/protocol'
import type { EditorController } from '../state/useEditorState'
import { ContextMenu, useContextMenu, type ContextMenuEntry } from '../components/ContextMenu'
import { Icon, type IconName } from '../components/Icon'

const levelIcons: Record<ConsoleLevel, IconName> = {
  info: 'info',
  warning: 'warning',
  error: 'error',
}

export function ConsolePanel({ controller }: { controller: EditorController }) {
  const [query, setQuery] = useState('')
  const [levels, setLevels] = useState<Set<ConsoleLevel>>(new Set(['info', 'warning', 'error']))
  const [collapse, setCollapse] = useState(false)
  const [selectedId, setSelectedId] = useState<string>()
  const [copyError, setCopyError] = useState<string>()
  const contextMenu = useContextMenu()
  const entries = useMemo(() => {
    const normalized = query.trim().toLocaleLowerCase()
    const filtered = controller.state.project.console.filter((entry) => levels.has(entry.level) && (!normalized || `${entry.message} ${entry.source}`.toLocaleLowerCase().includes(normalized)))
    if (!collapse) return filtered.map((entry) => ({ entry, count: 1 }))
    const groups = new Map<string, { entry: typeof filtered[number]; count: number }>()
    for (const entry of filtered) {
      const key = `${entry.level}:${entry.source}:${entry.message}`
      const existing = groups.get(key)
      if (existing) existing.count += 1
      else groups.set(key, { entry, count: 1 })
    }
    return [...groups.values()]
  }, [collapse, controller.state.project.console, levels, query])

  const toggleLevel = (level: ConsoleLevel) => {
    setLevels((current) => {
      const next = new Set(current)
      if (next.has(level)) next.delete(level)
      else next.add(level)
      return next
    })
  }
  const count = (level: ConsoleLevel) => controller.state.project.console.filter((entry) => entry.level === level).length
  const selected = controller.state.project.console.find((entry) => entry.id === selectedId)
  const clipboardAvailable = typeof navigator !== 'undefined'
    && typeof navigator.clipboard?.writeText === 'function'

  const clearConsole = () => {
    setSelectedId(undefined)
    void controller.invoke('console.clear', {})
  }

  const copyMessage = async (message: string) => {
    if (!clipboardAvailable) {
      setCopyError('The browser clipboard is unavailable in this editor host.')
      return
    }
    try {
      await navigator.clipboard.writeText(message)
      setCopyError(undefined)
    } catch (error) {
      setCopyError(`Could not copy the Console message: ${error instanceof Error ? error.message : String(error)}`)
    }
  }

  const commonMenuEntries = (): ContextMenuEntry[] => [
    {
      id: 'clear',
      label: 'Clear',
      icon: <Icon name="close" />,
      disabled: controller.state.project.console.length === 0,
      disabledReason: 'The Console is already empty.',
      onSelect: clearConsole,
    },
    {
      id: 'export',
      label: 'Export',
      icon: <Icon name="forward" />,
      onSelect: () => { void controller.invoke('console.export', {}) },
    },
    { type: 'separator', id: 'display-options' },
    {
      id: 'collapse',
      label: 'Collapse',
      checked: collapse,
      onSelect: () => setCollapse((current) => !current),
    },
    {
      id: 'levels',
      label: 'Log Levels',
      icon: <Icon name="filter" />,
      children: (['info', 'warning', 'error'] as const).map((level) => ({
        id: `level-${level}`,
        label: level === 'info' ? 'Info' : level === 'warning' ? 'Warnings' : 'Errors',
        icon: <Icon name={levelIcons[level]} />,
        checked: levels.has(level),
        onSelect: () => toggleLevel(level),
      })),
    },
  ]

  const entryMenuEntries = (entry: ConsoleEntry): ContextMenuEntry[] => {
    const actions: ContextMenuEntry[] = [
      {
        id: 'copy-message',
        label: 'Copy Message',
        icon: <Icon name="console" />,
        disabled: !clipboardAvailable,
        disabledReason: 'The browser clipboard is unavailable in this editor host.',
        onSelect: () => copyMessage(entry.message),
      },
    ]
    if (entry.entity) {
      actions.push({
        id: 'select-entity',
        label: 'Select Entity',
        icon: <Icon name="hierarchy" />,
        onSelect: () => { void controller.invoke('scene.select', { entityId: entry.entity, entityIds: [entry.entity!] }) },
      })
    }
    actions.push({ type: 'separator', id: 'entry-actions' }, ...commonMenuEntries())
    return actions
  }

  return (
    <div className="console-panel panel-column">
      <div className="console-toolbar">
        <button type="button" onClick={clearConsole}>Clear</button>
        <button className={collapse ? 'active' : ''} type="button" onClick={() => {
          setCollapse(!collapse)
        }}>Collapse</button>
        <button type="button" onClick={() => void controller.invoke('console.export', {})}>Export</button>
        <div className="console-toolbar-spacer" />
        <div className="compact-search"><Icon name="search" /><input value={query} placeholder="Filter console" onChange={(event) => setQuery(event.target.value)} /></div>
        <button className={levels.has('info') ? 'level-toggle active' : 'level-toggle'} type="button" onClick={() => toggleLevel('info')}><Icon name="info" /><span>{count('info')}</span></button>
        <button className={levels.has('warning') ? 'level-toggle active warning' : 'level-toggle warning'} type="button" onClick={() => toggleLevel('warning')}><Icon name="warning" /><span>{count('warning')}</span></button>
        <button className={levels.has('error') ? 'level-toggle active error' : 'level-toggle error'} type="button" onClick={() => toggleLevel('error')}><Icon name="error" /><span>{count('error')}</span></button>
      </div>
      {copyError && <div className="console-command-error" role="alert"><span>{copyError}</span><button type="button" aria-label="Dismiss clipboard error" onClick={() => setCopyError(undefined)}>×</button></div>}
      <div className="console-content">
        <div
          className="console-entries panel-scroll"
          onContextMenu={(event) => contextMenu.openContextMenu(event, commonMenuEntries(), { ariaLabel: 'Console menu' })}
        >
          {entries.map(({ entry, count: repeated }) => (
            <button
              className={`console-entry level-${entry.level} ${selectedId === entry.id ? 'selected' : ''}`}
              type="button"
              key={entry.id}
              onClick={() => setSelectedId(entry.id)}
              onContextMenu={(event) => {
                setSelectedId(entry.id)
                contextMenu.openContextMenu(event, entryMenuEntries(entry), { ariaLabel: 'Console entry menu' })
              }}
            >
              <Icon name={levelIcons[entry.level]} />
              <span className="console-message">{entry.message}</span>
              <span className="console-source">{entry.source}</span>
              {repeated > 1 && <span className="console-count">{repeated}</span>}
              <time>{entry.timestamp}</time>
            </button>
          ))}
          {entries.length === 0 && <div className="panel-empty"><Icon name="console" /><span>The Console has no matching messages</span></div>}
        </div>
        {selected && (
          <div className="console-details panel-scroll">
            <strong>{selected.source}</strong>
            <small>{selected.code}</small>
            <p>{selected.message}</p>
            {selected.path && <pre>{selected.path}</pre>}
            {selected.suggestedAction && <p>{selected.suggestedAction}</p>}
          </div>
        )}
      </div>
      <ContextMenu request={contextMenu.request} onClose={contextMenu.closeContextMenu} />
    </div>
  )
}
