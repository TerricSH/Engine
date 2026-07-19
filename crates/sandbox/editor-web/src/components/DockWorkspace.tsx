import { useEffect, useRef, type CSSProperties } from 'react'
import type { DockLayoutController, DockZoneId, PanelId } from '../layout/dockLayout'
import { PANEL_TITLES } from '../layout/dockLayout'
import type { EditorController } from '../state/useEditorState'
import { AnimationPanel } from '../panels/AnimationPanel'
import { BuildPanel } from '../panels/BuildPanel'
import { ConsolePanel } from '../panels/ConsolePanel'
import { HierarchyPanel } from '../panels/HierarchyPanel'
import { InspectorPanel } from '../panels/InspectorPanel'
import { MaterialPanel } from '../panels/MaterialPanel'
import { ProfilerPanel } from '../panels/ProfilerPanel'
import { TerrainPanel } from '../panels/TerrainPanel'
import { ProjectPanel } from '../panels/ProjectPanel'
import { SettingsPanel } from '../panels/SettingsPanel'
import {
  ContextMenu,
  useContextMenu,
  type ContextMenuEntry,
  type ContextMenuTriggerEvent,
} from './ContextMenu'
import { Icon } from './Icon'
import { ViewportPanel } from './ViewportPanel'

function ResizeHandle({ side, onResize }: { side: 'left' | 'right' | 'bottom'; onResize(side: 'left' | 'right' | 'bottom', delta: number): void }) {
  const startRef = useRef<{ x: number; y: number } | undefined>(undefined)
  useEffect(() => {
    const move = (event: PointerEvent) => {
      const start = startRef.current
      if (!start) return
      const delta = side === 'bottom' ? event.clientY - start.y : event.clientX - start.x
      startRef.current = { x: event.clientX, y: event.clientY }
      onResize(side, delta)
    }
    const stop = () => {
      startRef.current = undefined
      document.body.classList.remove('is-resizing')
    }
    window.addEventListener('pointermove', move)
    window.addEventListener('pointerup', stop)
    return () => {
      window.removeEventListener('pointermove', move)
      window.removeEventListener('pointerup', stop)
    }
  }, [onResize, side])
  return (
    <div
      className={`dock-resizer resizer-${side}`}
      onPointerDown={(event) => {
        event.currentTarget.setPointerCapture(event.pointerId)
        startRef.current = { x: event.clientX, y: event.clientY }
        document.body.classList.add('is-resizing')
      }}
    />
  )
}

function PanelContent({ panelId, zoneId, controller, nativeViewportKey }: { panelId: PanelId; zoneId: DockZoneId; controller: EditorController; nativeViewportKey?: string }) {
  switch (panelId) {
    case 'hierarchy': return <HierarchyPanel controller={controller} />
    case 'inspector': return <InspectorPanel controller={controller} />
    case 'project': return <ProjectPanel controller={controller} />
    case 'console': return <ConsolePanel controller={controller} />
    case 'material': return <MaterialPanel controller={controller} />
    case 'animation': return <AnimationPanel controller={controller} />
    case 'profiler': return <ProfilerPanel controller={controller} />
    case 'terrain': return <TerrainPanel controller={controller} />
    case 'build': return <BuildPanel controller={controller} />
    case 'settings': return <SettingsPanel controller={controller} />
    case 'scene':
    case 'game':
      return (
        <ViewportPanel
          viewport={panelId}
          active={nativeViewportKey === `${zoneId}:${panelId}`}
          bridgeAvailable={controller.state.bridgeAvailable}
          project={controller.state.project}
          onFocusSelection={() => void controller.invoke('viewport.focusSelection', {})}
        />
      )
  }
}

interface DockZoneProps {
  zoneId: DockZoneId
  controller: EditorController
  dock: DockLayoutController
  nativeViewportKey?: string
}

function DockZone({ zoneId, controller, dock, nativeViewportKey }: DockZoneProps) {
  const panelMenu = useContextMenu()
  const zone = dock.layout.zones[zoneId]
  const active = zone.panels.includes(zone.active) ? zone.active : zone.panels[0]
  const nativeViewport = active === 'scene' || active === 'game'
  const maximized = dock.layout.maximizedZone === zoneId

  const menuEntries = (panelId: PanelId): readonly ContextMenuEntry[] => [
    {
      id: 'close',
      label: `Close ${PANEL_TITLES[panelId]}`,
      icon: <Icon name="close" />,
      onSelect: () => dock.close(zoneId, panelId),
    },
    { type: 'separator', id: 'window-actions' },
    {
      id: 'maximize',
      label: maximized ? 'Restore Panel' : 'Maximize Panel',
      icon: <Icon name="maximize" />,
      onSelect: () => dock.toggleMaximized(zoneId),
    },
    {
      id: 'collapse',
      label: 'Collapse Panel',
      icon: <Icon name="collapse" />,
      onSelect: () => dock.toggleCollapsed(zoneId),
    },
    { type: 'separator', id: 'panel-list' },
    {
      id: 'open-panel',
      label: 'Open Panel',
      icon: <Icon name="layout" />,
      children: (Object.keys(PANEL_TITLES) as PanelId[]).map((candidate) => ({
        id: `open-${candidate}`,
        label: PANEL_TITLES[candidate],
        checked: (Object.keys(dock.layout.zones) as DockZoneId[])
          .some((candidateZone) => dock.layout.zones[candidateZone].panels.includes(candidate)),
        onSelect: () => dock.show(candidate, zoneId),
      })),
    },
    { type: 'separator', id: 'layout-actions' },
    {
      id: 'reset-layout',
      label: 'Reset Layout',
      icon: <Icon name="refresh" />,
      onSelect: dock.reset,
    },
  ]

  const openPanelMenu = (event: ContextMenuTriggerEvent, panelId: PanelId | undefined) => {
    if (!panelId) return
    panelMenu.openContextMenu(event, menuEntries(panelId), {
      ariaLabel: `${PANEL_TITLES[panelId]} panel menu`,
    })
  }

  if (zone.collapsed || zone.panels.length === 0) {
    return (
      <button className={`collapsed-dock collapsed-${zoneId}`} type="button" title={`Restore ${zoneId} dock`} onClick={() => dock.toggleCollapsed(zoneId)}>
        <Icon name="collapse" />
      </button>
    )
  }
  return (
    <section
      className={`dock-zone zone-${zoneId} ${nativeViewport ? 'has-native-viewport' : ''} ${maximized ? 'maximized' : ''}`}
      onDragOver={(event) => {
        if (Array.from(event.dataTransfer.types).includes('application/x-engine-panel')) event.preventDefault()
      }}
      onDrop={(event) => {
        const panelId = event.dataTransfer.getData('application/x-engine-panel') as PanelId
        if (panelId) dock.move(panelId, zoneId)
      }}
    >
      <header
        className="dock-tabbar"
        onContextMenu={(event) => openPanelMenu(event, active)}
        onDoubleClick={() => dock.toggleMaximized(zoneId)}
      >
        <div className="dock-tabs">
          {zone.panels.map((panelId) => (
            <button
              className={active === panelId ? 'dock-tab active' : 'dock-tab'}
              type="button"
              key={panelId}
              draggable
              onDragStart={(event) => {
                event.dataTransfer.setData('application/x-engine-panel', panelId)
                event.dataTransfer.effectAllowed = 'move'
              }}
              onClick={() => dock.activate(zoneId, panelId)}
              onContextMenu={(event) => {
                dock.activate(zoneId, panelId)
                openPanelMenu(event, panelId)
              }}
            >
              <span>{PANEL_TITLES[panelId]}</span>
              <span
                className="tab-close"
                role="button"
                tabIndex={0}
                aria-label={`Close ${PANEL_TITLES[panelId]}`}
                onClick={(event) => { event.stopPropagation(); dock.close(zoneId, panelId) }}
                onKeyDown={(event) => { if (event.key === 'Enter') dock.close(zoneId, panelId) }}
              >×</span>
            </button>
          ))}
        </div>
        <button className="dock-header-button" type="button" title={maximized ? 'Restore panel' : 'Maximize panel'} onClick={() => dock.toggleMaximized(zoneId)}><Icon name="maximize" /></button>
        <button className="dock-header-button" type="button" title="Collapse panel" onClick={() => dock.toggleCollapsed(zoneId)}><Icon name="collapse" /></button>
        <button
          aria-expanded={Boolean(panelMenu.request)}
          aria-haspopup="menu"
          className={panelMenu.request ? 'dock-header-button active' : 'dock-header-button'}
          onClick={(event) => openPanelMenu(event, active)}
          title="Panel menu"
          type="button"
        >⋮</button>
      </header>
      <div className="dock-zone-body">
        {active && <PanelContent panelId={active} zoneId={zoneId} controller={controller} nativeViewportKey={nativeViewportKey} />}
      </div>
      <ContextMenu request={panelMenu.request} onClose={panelMenu.closeContextMenu} />
    </section>
  )
}

export function DockWorkspace({ controller, dock }: { controller: EditorController; dock: DockLayoutController }) {
  const { layout } = dock
  const viewportZoneOrder: DockZoneId[] = layout.maximizedZone ? [layout.maximizedZone] : ['center', 'left', 'right', 'bottom']
  const viewportLocation = viewportZoneOrder
    .filter((zoneId) => !layout.zones[zoneId].collapsed)
    .map((zoneId) => [zoneId, layout.zones[zoneId].active] as const)
    .find(([, panelId]) => panelId === 'scene' || panelId === 'game')
  const nativeViewportKey = viewportLocation ? `${viewportLocation[0]}:${viewportLocation[1]}` : undefined
  const style = {
    '--left-width': `${layout.zones.left.collapsed ? 26 : layout.leftWidth}px`,
    '--right-width': `${layout.zones.right.collapsed ? 26 : layout.rightWidth}px`,
    '--bottom-height': `${layout.zones.bottom.collapsed ? 26 : layout.bottomHeight}px`,
  } as CSSProperties
  return (
    <main className={`dock-workspace ${layout.maximizedZone ? `has-maximized maximize-${layout.maximizedZone}` : ''}`} style={style}>
      <DockZone zoneId="left" controller={controller} dock={dock} nativeViewportKey={nativeViewportKey} />
      {!layout.zones.left.collapsed && !layout.maximizedZone && <ResizeHandle side="left" onResize={dock.resize} />}
      <DockZone zoneId="center" controller={controller} dock={dock} nativeViewportKey={nativeViewportKey} />
      {!layout.zones.right.collapsed && !layout.maximizedZone && <ResizeHandle side="right" onResize={dock.resize} />}
      <DockZone zoneId="right" controller={controller} dock={dock} nativeViewportKey={nativeViewportKey} />
      {!layout.zones.bottom.collapsed && !layout.maximizedZone && <ResizeHandle side="bottom" onResize={dock.resize} />}
      <DockZone zoneId="bottom" controller={controller} dock={dock} nativeViewportKey={nativeViewportKey} />
    </main>
  )
}
