import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { engineBridge } from '../bridge/engineBridge'
import type {
  FrameStatsSnapshot,
  InputModifiers,
  ProjectSnapshot,
  SceneCameraSnapshot,
  ScreenRect,
  ViewportInput,
} from '../bridge/protocol'
import { ContextMenu, useContextMenu, type ContextMenuEntry, type ContextMenuTriggerEvent } from './ContextMenu'
import { Icon } from './Icon'

const DEFAULT_SCENE_CAMERA: SceneCameraSnapshot = {
  pitch: 20,
  yaw: 45,
  distance: 10,
  target: [0, 0, 0],
  orthographic: false,
  speed: 5,
}

const CAMERA_SPEEDS = [0.5, 1, 2, 5, 10, 20, 50]
const GAME_ASPECTS = [
  { id: 'free', label: 'Free', ratio: undefined },
  { id: '16:9', label: '16:9', ratio: 16 / 9 },
  { id: '16:10', label: '16:10', ratio: 16 / 10 },
  { id: '4:3', label: '4:3', ratio: 4 / 3 },
] as const
type GameAspect = (typeof GAME_ASPECTS)[number]['id']

function modifiers(event: MouseEvent | PointerEvent | WheelEvent | KeyboardEvent): InputModifiers {
  return { alt: event.altKey, control: event.ctrlKey, meta: event.metaKey, shift: event.shiftKey }
}

function ViewportStats({ stats }: { stats: FrameStatsSnapshot }) {
  const fps = stats.frameTimeMs > 0 ? Math.round(1000 / stats.frameTimeMs) : 0
  return (
    <div className="viewport-stats" aria-live="polite">
      <strong>Rendering Stats</strong>
      <span>{fps} FPS / {stats.frameTimeMs.toFixed(2)} ms</span>
      <span>{stats.drawCalls.toLocaleString()} draw calls</span>
      <span>{stats.triangles.toLocaleString()} triangles</span>
      <span>{stats.physicsBodies.toLocaleString()} physics bodies</span>
      <span>{stats.assetCount.toLocaleString()} loaded assets</span>
    </div>
  )
}

interface ViewportSurfaceProps {
  viewport: 'scene' | 'game'
  active: boolean
  bridgeAvailable: boolean
  sessionId: string
  aspectRatio?: number
  stats?: FrameStatsSnapshot
  onOpenContextMenu(event: ContextMenuTriggerEvent): void
  onFocusSelection(): void
}

function ViewportSurface({ viewport, active, bridgeAvailable, sessionId, aspectRatio, stats, onOpenContextMenu, onFocusSelection }: ViewportSurfaceProps) {
  const stageRef = useRef<HTMLDivElement>(null)
  const elementRef = useRef<HTMLDivElement>(null)
  const frameRef = useRef<number | undefined>(undefined)
  const pendingMoveRef = useRef<ViewportInput | undefined>(undefined)
  const pendingWheelRef = useRef<Extract<ViewportInput, { type: 'wheel' }> | undefined>(undefined)
  const pendingRightPointerRef = useRef<{
    pointerId: number
    startClientX: number
    startClientY: number
    input: ViewportInput
    sent: boolean
  } | undefined>(undefined)
  const suppressNextContextMenuRef = useRef(false)
  const [stageSize, setStageSize] = useState({ width: 0, height: 0 })

  const notifyInput = useCallback((event: ViewportInput) => {
    engineBridge.notify('viewport.input', { viewport, event })
  }, [viewport])

  const flushInput = useCallback(() => {
    if (frameRef.current !== undefined) window.cancelAnimationFrame(frameRef.current)
    frameRef.current = undefined
    if (pendingMoveRef.current) notifyInput(pendingMoveRef.current)
    if (pendingWheelRef.current) notifyInput(pendingWheelRef.current)
    pendingMoveRef.current = undefined
    pendingWheelRef.current = undefined
  }, [notifyInput])

  const scheduleInput = useCallback(() => {
    if (frameRef.current === undefined) frameRef.current = window.requestAnimationFrame(flushInput)
  }, [flushInput])

  const sendBounds = useCallback(() => {
    const bounds = elementRef.current?.getBoundingClientRect()
    const rect: ScreenRect = bounds
      ? { x: bounds.x, y: bounds.y, width: bounds.width, height: bounds.height }
      : { x: 0, y: 0, width: 0, height: 0 }
    engineBridge.notify('viewport.bounds', {
      viewport,
      rect,
      visible: active && Boolean(bounds && bounds.width > 0 && bounds.height > 0),
    })
  }, [active, viewport])

  useEffect(() => {
    const stage = stageRef.current
    if (!stage || aspectRatio === undefined) {
      setStageSize({ width: 0, height: 0 })
      return
    }
    const update = () => {
      const bounds = stage.getBoundingClientRect()
      const width = Math.max(0, bounds.width)
      const height = Math.max(0, bounds.height)
      if (width <= 0 || height <= 0) {
        setStageSize({ width: 0, height: 0 })
      } else if (width / height > aspectRatio) {
        setStageSize({ width: height * aspectRatio, height })
      } else {
        setStageSize({ width, height: width / aspectRatio })
      }
    }
    const observer = new ResizeObserver(update)
    observer.observe(stage)
    update()
    return () => observer.disconnect()
  }, [aspectRatio])

  useEffect(() => {
    const element = elementRef.current
    if (!element || !active || !sessionId) return
    let boundsFrame: number | undefined
    const scheduleBounds = () => {
      if (boundsFrame === undefined) boundsFrame = window.requestAnimationFrame(() => { boundsFrame = undefined; sendBounds() })
    }
    const resizeObserver = new ResizeObserver(scheduleBounds)
    resizeObserver.observe(element)
    window.addEventListener('resize', scheduleBounds)
    window.addEventListener('scroll', scheduleBounds, true)
    scheduleBounds()
    return () => {
      if (boundsFrame !== undefined) window.cancelAnimationFrame(boundsFrame)
      resizeObserver.disconnect()
      window.removeEventListener('resize', scheduleBounds)
      window.removeEventListener('scroll', scheduleBounds, true)
      engineBridge.notify('viewport.bounds', { viewport, rect: { x: 0, y: 0, width: 0, height: 0 }, visible: false })
    }
  }, [active, sendBounds, sessionId, viewport])

  useEffect(() => () => {
    if (frameRef.current !== undefined) window.cancelAnimationFrame(frameRef.current)
  }, [])

  const localPosition = (event: React.PointerEvent | React.WheelEvent) => {
    const bounds = elementRef.current?.getBoundingClientRect()
    return { x: event.clientX - (bounds?.left ?? 0), y: event.clientY - (bounds?.top ?? 0) }
  }

  const surfaceStyle = aspectRatio !== undefined && stageSize.width > 0 && stageSize.height > 0
    ? { width: `${stageSize.width}px`, height: `${stageSize.height}px` }
    : undefined

  return (
    <div ref={stageRef} className={`native-viewport-stage ${aspectRatio === undefined ? '' : 'letterboxed'}`}>
      <div
        ref={elementRef}
        className="native-viewport-surface"
        style={surfaceStyle}
        data-native-viewport={active ? viewport : undefined}
        tabIndex={active ? 0 : -1}
        aria-label={`${viewport} viewport`}
        onContextMenu={(event) => {
          const rightPointer = pendingRightPointerRef.current
          if (rightPointer?.sent || suppressNextContextMenuRef.current) {
            event.preventDefault()
            event.stopPropagation()
            suppressNextContextMenuRef.current = false
            return
          }
          if (rightPointer) {
            pendingRightPointerRef.current = undefined
            if (event.currentTarget.hasPointerCapture(rightPointer.pointerId)) {
              event.currentTarget.releasePointerCapture(rightPointer.pointerId)
            }
          }
          onOpenContextMenu(event)
        }}
        onFocus={() => active && notifyInput({ type: 'focus' })}
        onBlur={() => {
          pendingRightPointerRef.current = undefined
          if (active) notifyInput({ type: 'blur' })
        }}
        onPointerDown={(event) => {
          if (!active) return
          flushInput()
          event.currentTarget.focus({ preventScroll: true })
          event.currentTarget.setPointerCapture(event.pointerId)
          if (event.button === 2) {
            pendingRightPointerRef.current = {
              pointerId: event.pointerId,
              startClientX: event.clientX,
              startClientY: event.clientY,
              input: { type: 'pointerDown', pointerId: event.pointerId, ...localPosition(event), button: event.button, buttons: event.buttons, modifiers: modifiers(event.nativeEvent) },
              sent: false,
            }
            return
          }
          notifyInput({ type: 'pointerDown', pointerId: event.pointerId, ...localPosition(event), button: event.button, buttons: event.buttons, modifiers: modifiers(event.nativeEvent) })
        }}
        onPointerMove={(event) => {
          if (!active) return
          const rightPointer = pendingRightPointerRef.current
          if (rightPointer?.pointerId === event.pointerId && !rightPointer.sent) {
            const moved = Math.hypot(
              event.clientX - rightPointer.startClientX,
              event.clientY - rightPointer.startClientY,
            )
            if (moved < 4) return
            rightPointer.sent = true
            notifyInput(rightPointer.input)
          }
          pendingMoveRef.current = { type: 'pointerMove', pointerId: event.pointerId, ...localPosition(event), button: event.button, buttons: event.buttons, modifiers: modifiers(event.nativeEvent) }
          scheduleInput()
        }}
        onPointerUp={(event) => {
          if (!active) return
          const rightPointer = pendingRightPointerRef.current
          if (event.button === 2) {
            pendingRightPointerRef.current = undefined
            if (rightPointer?.sent) {
              flushInput()
              notifyInput({ type: 'pointerUp', pointerId: event.pointerId, ...localPosition(event), button: event.button, buttons: event.buttons, modifiers: modifiers(event.nativeEvent) })
              suppressNextContextMenuRef.current = true
              window.setTimeout(() => { suppressNextContextMenuRef.current = false }, 300)
            }
            if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
            return
          }
          flushInput()
          notifyInput({ type: 'pointerUp', pointerId: event.pointerId, ...localPosition(event), button: event.button, buttons: event.buttons, modifiers: modifiers(event.nativeEvent) })
          if (event.currentTarget.hasPointerCapture(event.pointerId)) event.currentTarget.releasePointerCapture(event.pointerId)
        }}
        onPointerCancel={(event) => {
          const rightPointer = pendingRightPointerRef.current
          if (rightPointer?.pointerId === event.pointerId) {
            pendingRightPointerRef.current = undefined
            if (active && rightPointer.sent) notifyInput({ type: 'pointerCancel', pointerId: event.pointerId })
            return
          }
          if (active) notifyInput({ type: 'pointerCancel', pointerId: event.pointerId })
        }}
        onWheel={(event) => {
          if (!active) return
          event.preventDefault()
          const previous = pendingWheelRef.current
          pendingWheelRef.current = {
            type: 'wheel', ...localPosition(event),
            deltaX: (previous?.deltaX ?? 0) + event.deltaX,
            deltaY: (previous?.deltaY ?? 0) + event.deltaY,
            deltaMode: event.deltaMode,
            modifiers: modifiers(event.nativeEvent),
          }
          scheduleInput()
        }}
        onKeyDown={(event) => {
          const hostOnly = event.key === 'F5' || ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === 'k')
          const focusSelection = viewport === 'scene'
            && event.code === 'KeyF'
            && !event.altKey
            && !event.ctrlKey
            && !event.metaKey
            && !event.shiftKey
          if (active && focusSelection && !event.repeat) {
            event.preventDefault()
            onFocusSelection()
            return
          }
          if (active && !hostOnly) notifyInput({ type: 'keyDown', key: event.key, code: event.code, repeat: event.repeat, modifiers: modifiers(event.nativeEvent) })
        }}
        onKeyUp={(event) => {
          const hostOnly = event.key === 'F5' || ((event.ctrlKey || event.metaKey) && event.key.toLocaleLowerCase() === 'k')
          const focusSelection = viewport === 'scene'
            && event.code === 'KeyF'
            && !event.altKey
            && !event.ctrlKey
            && !event.metaKey
            && !event.shiftKey
          if (active && focusSelection) {
            event.preventDefault()
            return
          }
          if (active && !hostOnly) notifyInput({ type: 'keyUp', key: event.key, code: event.code, repeat: event.repeat, modifiers: modifiers(event.nativeEvent) })
        }}
      >
        {!bridgeAvailable && <div className="viewport-host-unavailable"><Icon name="warning" /><span>Native viewport host is not connected</span></div>}
        {bridgeAvailable && !active && <div className="viewport-host-unavailable"><Icon name="info" /><span>Another native viewport is active</span></div>}
        {stats && <ViewportStats stats={stats} />}
      </div>
    </div>
  )
}

interface ViewportPanelProps extends Pick<ViewportSurfaceProps, 'viewport' | 'active' | 'bridgeAvailable'> {
  project: ProjectSnapshot
  onFocusSelection(): void
}

export function ViewportPanel({ viewport, active, bridgeAvailable, project, onFocusSelection }: ViewportPanelProps) {
  const [camera, setCamera] = useState<SceneCameraSnapshot>(DEFAULT_SCENE_CAMERA)
  const [gizmosVisible, setGizmosVisible] = useState(true)
  const [gameAspect, setGameAspect] = useState<GameAspect>('free')
  const [statsVisible, setStatsVisible] = useState(false)
  const contextMenu = useContextMenu()

  useEffect(() => {
    setCamera(project.viewport.sceneCamera)
    setGizmosVisible(project.viewport.gizmosVisible)
  }, [project.viewport])

  const setSceneCamera = useCallback((update: Partial<SceneCameraSnapshot>) => {
    setCamera((current) => {
      const next = { ...current, ...update }
      void engineBridge.invoke('viewport.setCamera', next).catch((error: unknown) => {
        console.error('Could not update Scene camera', error)
      })
      return next
    })
  }, [])

  const setGizmos = useCallback((visible: boolean) => {
    setGizmosVisible(visible)
    void engineBridge.invoke('viewport.setGizmos', { visible }).catch((error: unknown) => {
      console.error('Could not update gizmo visibility', error)
    })
  }, [])

  const planar2D = camera.orthographic
    && Math.abs(camera.pitch) < 0.01
    && Math.abs(camera.yaw - 90) < 0.01
  const selectedAspect = useMemo(
    () => GAME_ASPECTS.find((option) => option.id === gameAspect) ?? GAME_ASPECTS[0],
    [gameAspect],
  )
  const speedOptions = CAMERA_SPEEDS.includes(camera.speed)
    ? CAMERA_SPEEDS
    : [...CAMERA_SPEEDS, camera.speed].sort((left, right) => left - right)

  const viewportMenuEntries: readonly ContextMenuEntry[] = viewport === 'scene'
    ? [
      {
        id: 'focus-selection',
        label: 'Focus Selection',
        icon: <Icon name="maximize" />,
        shortcut: 'F',
        disabled: !project.capabilities.hasSelection,
        disabledReason: 'Select a Scene entity first.',
        onSelect: onFocusSelection,
      },
      { type: 'separator', id: 'scene-view' },
      {
        id: '2d-mode',
        label: '2D Mode',
        icon: <Icon name="rect" />,
        checked: planar2D,
        onSelect: () => setSceneCamera(planar2D
          ? { pitch: 20, yaw: 45, orthographic: false }
          : { pitch: 0, yaw: 90, orthographic: true }),
      },
      {
        id: 'projection',
        label: 'Projection',
        icon: <Icon name="camera" />,
        children: [
          {
            id: 'perspective',
            label: 'Perspective',
            checked: !camera.orthographic,
            onSelect: () => setSceneCamera({ orthographic: false }),
          },
          {
            id: 'orthographic',
            label: 'Orthographic',
            checked: camera.orthographic,
            onSelect: () => setSceneCamera({ orthographic: true }),
          },
        ],
      },
      { type: 'separator', id: 'scene-overlays' },
      {
        id: 'gizmos',
        label: 'Gizmos',
        icon: <Icon name="eye" />,
        checked: gizmosVisible,
        onSelect: () => setGizmos(!gizmosVisible),
      },
      {
        id: 'stats',
        label: 'Stats',
        icon: <Icon name="profiler" />,
        checked: statsVisible,
        onSelect: () => setStatsVisible((visible) => !visible),
      },
    ]
    : [
      {
        id: 'stats',
        label: 'Stats',
        icon: <Icon name="profiler" />,
        checked: statsVisible,
        onSelect: () => setStatsVisible((visible) => !visible),
      },
      {
        id: 'aspect-ratio',
        label: 'Aspect Ratio',
        icon: <Icon name="game" />,
        children: GAME_ASPECTS.map((option) => ({
          id: `aspect-${option.id}`,
          label: option.label,
          checked: gameAspect === option.id,
          onSelect: () => setGameAspect(option.id),
        })),
      },
    ]

  return (
    <div className="viewport-panel">
      <div className="viewport-toolbar">
        {viewport === 'scene' ? <>
          <span className="viewport-mode-label" title="Native forward shaded rendering"><span className="sphere-icon" /> Shaded</span>
          <button
            type="button"
            className={planar2D ? 'active' : ''}
            onClick={() => setSceneCamera(planar2D
              ? { pitch: 20, yaw: 45, orthographic: false }
              : { pitch: 0, yaw: 90, orthographic: true })}
            title={planar2D ? 'Switch to a perspective 3D view' : 'Align to the XY plane in orthographic 2D'}
          >{planar2D ? '2D' : '3D'}</button>
          <button
            type="button"
            className={camera.orthographic ? 'active' : ''}
            onClick={() => setSceneCamera({ orthographic: !camera.orthographic })}
            title="Toggle the native Scene camera projection"
          ><Icon name="camera" /> {camera.orthographic ? 'Orthographic' : 'Perspective'}</button>
          <label className="viewport-speed" title="Scene camera movement speed">
            Speed
            <select value={camera.speed} onChange={(event) => setSceneCamera({ speed: Number(event.target.value) })}>
              {speedOptions.map((speed) => <option value={speed} key={speed}>{speed}x</option>)}
            </select>
          </label>
          <div className="viewport-toolbar-spacer" />
          <button
            type="button"
            className={gizmosVisible ? 'active' : ''}
            aria-pressed={gizmosVisible}
            onClick={() => {
              setGizmos(!gizmosVisible)
            }}
            title="Show or hide native transform gizmos"
          ><Icon name="eye" /> Gizmos</button>
          <button type="button" onClick={onFocusSelection} title="Frame Selected (F)"><Icon name="maximize" /> Focus</button>
          <button type="button" className={statsVisible ? 'active' : ''} aria-pressed={statsVisible} onClick={() => setStatsVisible((visible) => !visible)}><Icon name="profiler" /> Stats</button>
        </> : <>
          <label className="viewport-aspect">
            Aspect
            <select value={gameAspect} onChange={(event) => setGameAspect(event.target.value as GameAspect)}>
              {GAME_ASPECTS.map((option) => <option value={option.id} key={option.id}>{option.label}</option>)}
            </select>
          </label>
          <div className="viewport-toolbar-spacer" />
          <button
            type="button"
            className={statsVisible ? 'active' : ''}
            aria-pressed={statsVisible}
            onClick={() => setStatsVisible((visible) => !visible)}
          ><Icon name="eye" /> Stats</button>
        </>}
      </div>
      <ViewportSurface
        viewport={viewport}
        active={active}
        bridgeAvailable={bridgeAvailable}
        sessionId={project.sessionId}
        aspectRatio={viewport === 'game' ? selectedAspect.ratio : undefined}
        stats={statsVisible ? project.performance.current : undefined}
        onOpenContextMenu={(event) => contextMenu.openContextMenu(event, viewportMenuEntries, {
          ariaLabel: `${viewport === 'scene' ? 'Scene' : 'Game'} viewport menu`,
        })}
        onFocusSelection={onFocusSelection}
      />
      <ContextMenu request={contextMenu.request} onClose={contextMenu.closeContextMenu} />
    </div>
  )
}
