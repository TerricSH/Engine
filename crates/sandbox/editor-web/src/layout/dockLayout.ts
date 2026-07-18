import { useCallback, useEffect, useRef, useState } from 'react'
import { engineBridge } from '../bridge/engineBridge'
import type { UiDockZone, UiPanelId } from '../bridge/protocol'

export type DockZoneId = UiDockZone
export type PanelId = UiPanelId

export interface DockZoneState {
  panels: PanelId[]
  active: PanelId
  collapsed: boolean
}

export interface DockLayoutState {
  zones: Record<DockZoneId, DockZoneState>
  leftWidth: number
  rightWidth: number
  bottomHeight: number
  maximizedZone?: DockZoneId
}

export const PANEL_TITLES: Record<PanelId, string> = {
  hierarchy: 'Hierarchy',
  scene: 'Scene',
  game: 'Game',
  inspector: 'Inspector',
  project: 'Project',
  console: 'Console',
  material: 'Material',
  animation: 'Animation',
  profiler: 'Profiler',
  build: 'Build',
  settings: 'Settings',
}

export const DEFAULT_DOCK_LAYOUT: DockLayoutState = {
  zones: {
    left: { panels: ['hierarchy'], active: 'hierarchy', collapsed: false },
    center: { panels: ['scene', 'game'], active: 'scene', collapsed: false },
    right: { panels: ['inspector', 'settings'], active: 'inspector', collapsed: false },
    bottom: { panels: ['project', 'console', 'material', 'animation', 'profiler', 'build'], active: 'project', collapsed: false },
  },
  leftWidth: 272,
  rightWidth: 326,
  bottomHeight: 260,
}

function isPanelId(value: unknown): value is PanelId {
  return typeof value === 'string' && value in PANEL_TITLES
}

function defaultDockLayout(): DockLayoutState {
  return {
    ...DEFAULT_DOCK_LAYOUT,
    zones: {
      left: { ...DEFAULT_DOCK_LAYOUT.zones.left, panels: [...DEFAULT_DOCK_LAYOUT.zones.left.panels] },
      center: { ...DEFAULT_DOCK_LAYOUT.zones.center, panels: [...DEFAULT_DOCK_LAYOUT.zones.center.panels] },
      right: { ...DEFAULT_DOCK_LAYOUT.zones.right, panels: [...DEFAULT_DOCK_LAYOUT.zones.right.panels] },
      bottom: { ...DEFAULT_DOCK_LAYOUT.zones.bottom, panels: [...DEFAULT_DOCK_LAYOUT.zones.bottom.panels] },
    },
  }
}

export function parseDockLayout(serializedLayout?: string): DockLayoutState {
  try {
    const parsed = JSON.parse(serializedLayout ?? '') as Partial<DockLayoutState>
    const zoneIds: DockZoneId[] = ['left', 'center', 'right', 'bottom']
    if (!parsed.zones || !zoneIds.every((zoneId) => {
      const zone = parsed.zones?.[zoneId]
      return Boolean(zone && Array.isArray(zone.panels) && zone.panels.every(isPanelId) && isPanelId(zone.active) && typeof zone.collapsed === 'boolean')
    })) {
      return defaultDockLayout()
    }
    return {
      zones: {
        left: { ...parsed.zones.left, panels: [...new Set(parsed.zones.left.panels)] },
        center: { ...parsed.zones.center, panels: [...new Set(parsed.zones.center.panels)] },
        right: { ...parsed.zones.right, panels: [...new Set(parsed.zones.right.panels)] },
        bottom: { ...parsed.zones.bottom, panels: [...new Set(parsed.zones.bottom.panels)] },
      },
      leftWidth: clamp(Number(parsed.leftWidth) || DEFAULT_DOCK_LAYOUT.leftWidth, 190, 520),
      rightWidth: clamp(Number(parsed.rightWidth) || DEFAULT_DOCK_LAYOUT.rightWidth, 240, 560),
      bottomHeight: clamp(Number(parsed.bottomHeight) || DEFAULT_DOCK_LAYOUT.bottomHeight, 150, 520),
      maximizedZone: parsed.maximizedZone && zoneIds.includes(parsed.maximizedZone) ? parsed.maximizedZone : undefined,
    }
  } catch {
    return defaultDockLayout()
  }
}

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.max(minimum, Math.min(maximum, value))
}

export interface DockLayoutController {
  layout: DockLayoutState
  activate(zoneId: DockZoneId, panelId: PanelId): void
  move(panelId: PanelId, targetZoneId: DockZoneId): void
  close(zoneId: DockZoneId, panelId: PanelId): void
  show(panelId: PanelId, preferredZone?: DockZoneId): void
  toggleCollapsed(zoneId: DockZoneId): void
  resize(side: 'left' | 'right' | 'bottom', delta: number): void
  reset(): void
  toggleMaximized(zoneId: DockZoneId): void
}

export function useDockLayout(sessionId?: string, projectLayout?: string): DockLayoutController {
  const [layout, setLayout] = useState(defaultDockLayout)
  const hydratedSession = useRef<string | undefined>(undefined)
  const lastPersistedLayout = useRef<string | undefined>(undefined)
  const skipNextPersist = useRef(false)

  useEffect(() => {
    if (!sessionId) {
      hydratedSession.current = undefined
      lastPersistedLayout.current = undefined
      skipNextPersist.current = true
      setLayout(defaultDockLayout())
      return
    }
    if (hydratedSession.current === sessionId) return
    const restored = parseDockLayout(projectLayout)
    hydratedSession.current = sessionId
    lastPersistedLayout.current = JSON.stringify(restored)
    skipNextPersist.current = true
    setLayout(restored)
  }, [projectLayout, sessionId])

  useEffect(() => {
    if (!sessionId || hydratedSession.current !== sessionId) return
    if (skipNextPersist.current) {
      skipNextPersist.current = false
      return
    }
    const serializedLayout = JSON.stringify(layout)
    if (serializedLayout === lastPersistedLayout.current) return
    const timeout = window.setTimeout(() => {
      if (hydratedSession.current === sessionId && engineBridge.connected) {
        void engineBridge.invoke('layout.persist', { serializedLayout }).then(() => {
          if (hydratedSession.current === sessionId) lastPersistedLayout.current = serializedLayout
        }).catch((error: unknown) => {
          console.error('Failed to persist editor layout', error)
        })
      }
    }, 400)
    return () => window.clearTimeout(timeout)
  }, [layout, sessionId])

  const activate = useCallback((zoneId: DockZoneId, panelId: PanelId) => {
    setLayout((current) => ({
      ...current,
      zones: {
        ...current.zones,
        [zoneId]: { ...current.zones[zoneId], active: panelId, collapsed: false },
      },
    }))
  }, [])

  const move = useCallback((panelId: PanelId, targetZoneId: DockZoneId) => {
    setLayout((current) => {
      let sourceZoneId: DockZoneId | undefined
      const zones = { ...current.zones }
      ;(Object.keys(zones) as DockZoneId[]).forEach((zoneId) => {
        if (zones[zoneId].panels.includes(panelId)) sourceZoneId = zoneId
      })

      if (sourceZoneId === targetZoneId) {
        return { ...current, zones: { ...zones, [targetZoneId]: { ...zones[targetZoneId], active: panelId } } }
      }

      if (sourceZoneId) {
        const source = zones[sourceZoneId]
        const panels = source.panels.filter((id) => id !== panelId)
        zones[sourceZoneId] = {
          ...source,
          panels,
          active: source.active === panelId ? (panels[0] ?? source.active) : source.active,
          collapsed: panels.length === 0 || source.collapsed,
        }
      }

      const target = zones[targetZoneId]
      zones[targetZoneId] = {
        ...target,
        panels: target.panels.includes(panelId) ? target.panels : [...target.panels, panelId],
        active: panelId,
        collapsed: false,
      }
      const sourceBecameEmpty = sourceZoneId ? zones[sourceZoneId].panels.length === 0 : false
      return {
        ...current,
        zones,
        maximizedZone: sourceBecameEmpty && current.maximizedZone === sourceZoneId ? undefined : current.maximizedZone,
      }
    })
  }, [])

  const close = useCallback((zoneId: DockZoneId, panelId: PanelId) => {
    setLayout((current) => {
      const zone = current.zones[zoneId]
      const panels = zone.panels.filter((id) => id !== panelId)
      return {
        ...current,
        zones: {
          ...current.zones,
          [zoneId]: {
            ...zone,
            panels,
            active: zone.active === panelId ? (panels[0] ?? zone.active) : zone.active,
            collapsed: panels.length === 0,
          },
        },
        maximizedZone: panels.length === 0 && current.maximizedZone === zoneId ? undefined : current.maximizedZone,
      }
    })
  }, [])

  const show = useCallback((panelId: PanelId, preferredZone: DockZoneId = 'bottom') => {
    setLayout((current) => {
      const existing = (Object.keys(current.zones) as DockZoneId[]).find((zoneId) => current.zones[zoneId].panels.includes(panelId))
      const zoneId = existing ?? preferredZone
      const zone = current.zones[zoneId]
      return {
        ...current,
        maximizedZone: undefined,
        zones: {
          ...current.zones,
          [zoneId]: {
            ...zone,
            panels: zone.panels.includes(panelId) ? zone.panels : [...zone.panels, panelId],
            active: panelId,
            collapsed: false,
          },
        },
      }
    })
  }, [])

  const toggleCollapsed = useCallback((zoneId: DockZoneId) => {
    setLayout((current) => {
      const collapsed = !current.zones[zoneId].collapsed
      return {
        ...current,
        zones: { ...current.zones, [zoneId]: { ...current.zones[zoneId], collapsed } },
        maximizedZone: collapsed && current.maximizedZone === zoneId ? undefined : current.maximizedZone,
      }
    })
  }, [])

  const resize = useCallback((side: 'left' | 'right' | 'bottom', delta: number) => {
    setLayout((current) => {
      if (side === 'left') return { ...current, leftWidth: clamp(current.leftWidth + delta, 190, 520) }
      if (side === 'right') return { ...current, rightWidth: clamp(current.rightWidth - delta, 240, 560) }
      return { ...current, bottomHeight: clamp(current.bottomHeight - delta, 150, 520) }
    })
  }, [])

  const reset = useCallback(() => setLayout(defaultDockLayout()), [])
  const toggleMaximized = useCallback((zoneId: DockZoneId) => setLayout((current) => ({ ...current, maximizedZone: current.maximizedZone === zoneId ? undefined : zoneId })), [])

  return { layout, activate, move, close, show, toggleCollapsed, resize, reset, toggleMaximized }
}
