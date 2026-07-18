import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
  type ReactNode,
  type RefObject,
} from 'react'
import { createPortal } from 'react-dom'
import {
  clampContextMenuPosition,
  preferredSubmenuPosition,
  type ContextMenuPosition as MenuPosition,
  type ContextMenuViewportBounds as ViewportBounds,
} from './ContextMenuGeometry'

export interface ContextMenuItem {
  type?: 'item'
  id: string
  label: ReactNode
  ariaLabel?: string
  icon?: ReactNode
  shortcut?: string
  checked?: boolean
  disabled?: boolean
  disabledReason?: string
  danger?: boolean
  onSelect?: () => unknown | Promise<unknown>
  children?: readonly ContextMenuEntry[]
}

export interface ContextMenuSeparator {
  type: 'separator'
  id?: string
}

export type ContextMenuEntry = ContextMenuItem | ContextMenuSeparator

export interface ContextMenuRequest {
  x: number
  y: number
  items: readonly ContextMenuEntry[]
  ariaLabel?: string
  restoreFocusTo?: HTMLElement | null
}

export interface ContextMenuOpenOptions {
  ariaLabel?: string
  restoreFocusTo?: HTMLElement | null
  stopPropagation?: boolean
}

export interface ContextMenuTriggerEvent {
  clientX: number
  clientY: number
  currentTarget?: EventTarget | null
  preventDefault(): void
  stopPropagation(): void
}

export interface ContextMenuProps {
  request: ContextMenuRequest | null
  onClose: () => void
}

const VIEWPORT_MARGIN = 6
const SUBMENU_OVERLAP = 3

function currentViewport(): ViewportBounds {
  const viewport = window.visualViewport
  if (viewport) {
    return {
      left: viewport.offsetLeft,
      top: viewport.offsetTop,
      width: viewport.width,
      height: viewport.height,
    }
  }
  return { left: 0, top: 0, width: window.innerWidth, height: window.innerHeight }
}

function isSeparator(entry: ContextMenuEntry): entry is ContextMenuSeparator {
  return entry.type === 'separator'
}

function firstEnabledIndex(items: readonly ContextMenuEntry[]): number {
  return items.findIndex((entry) => !isSeparator(entry) && !entry.disabled)
}

function lastEnabledIndex(items: readonly ContextMenuEntry[]): number {
  for (let index = items.length - 1; index >= 0; index -= 1) {
    const entry = items[index]
    if (entry && !isSeparator(entry) && !entry.disabled) return index
  }
  return -1
}

function adjacentEnabledIndex(
  items: readonly ContextMenuEntry[],
  currentIndex: number,
  direction: 1 | -1,
): number {
  if (items.length === 0) return -1
  for (let offset = 1; offset <= items.length; offset += 1) {
    const index = (currentIndex + direction * offset + items.length) % items.length
    const entry = items[index]
    if (entry && !isSeparator(entry) && !entry.disabled) return index
  }
  return -1
}

function hasChildren(item: ContextMenuItem): boolean {
  return Boolean(item.children?.length)
}

function focusElement(element: HTMLElement | null | undefined) {
  if (!element?.isConnected) return
  element.focus({ preventScroll: true })
}

export function useContextMenu() {
  const [request, setRequest] = useState<ContextMenuRequest | null>(null)
  const requestRef = useRef<ContextMenuRequest | null>(null)

  const openContextMenu = useCallback(
    (
      event: ContextMenuTriggerEvent,
      items: readonly ContextMenuEntry[],
      options: ContextMenuOpenOptions = {},
    ) => {
      event.preventDefault()
      if (options.stopPropagation !== false) event.stopPropagation()

      const activeElement = document.activeElement instanceof HTMLElement
        && document.activeElement !== document.body
        ? document.activeElement
        : null
      const eventTarget = event.currentTarget instanceof HTMLElement ? event.currentTarget : null
      const restoreFocusTo = options.restoreFocusTo !== undefined
        ? options.restoreFocusTo
        : activeElement ?? eventTarget
      const nextRequest: ContextMenuRequest = {
        x: event.clientX,
        y: event.clientY,
        items,
        ariaLabel: options.ariaLabel,
        restoreFocusTo,
      }
      requestRef.current = nextRequest
      setRequest(nextRequest)
    },
    [],
  )

  const closeContextMenu = useCallback(() => {
    const closingRequest = requestRef.current
    requestRef.current = null
    setRequest(null)

    window.requestAnimationFrame(() => {
      const activeElement = document.activeElement
      const focusWasReleased = activeElement === document.body
        || (activeElement instanceof HTMLElement && Boolean(activeElement.closest('.context-menu')))
      if (!requestRef.current && focusWasReleased) focusElement(closingRequest?.restoreFocusTo)
    })
  }, [])

  return { request, openContextMenu, closeContextMenu }
}

interface MenuLevelProps {
  items: readonly ContextMenuEntry[]
  ariaLabel: string
  onCloseAll: () => void
  parentItem?: HTMLElement | null
  onCloseSubmenu?: () => void
  onReturnFocus?: () => void
  autoFocus?: boolean
  rootRef?: RefObject<HTMLDivElement | null>
  depth?: number
  preferredPosition?: MenuPosition
}

function MenuLevel({
  items,
  ariaLabel,
  onCloseAll,
  parentItem,
  onCloseSubmenu,
  onReturnFocus,
  autoFocus = false,
  rootRef,
  depth = 0,
  preferredPosition,
}: MenuLevelProps) {
  const localMenuRef = useRef<HTMLDivElement>(null)
  const menuRef = rootRef ?? localMenuRef
  const itemRefs = useRef<Array<HTMLButtonElement | null>>([])
  const initialIndex = firstEnabledIndex(items)
  const [activeIndex, setActiveIndex] = useState(initialIndex)
  const [openSubmenuIndex, setOpenSubmenuIndex] = useState<number | null>(null)
  const [focusSubmenu, setFocusSubmenu] = useState(false)
  const [position, setPosition] = useState<MenuPosition>(() => preferredPosition ?? { left: 0, top: 0 })

  useEffect(() => {
    const activeEntry = items[activeIndex]
    if (!activeEntry || isSeparator(activeEntry) || activeEntry.disabled) {
      setActiveIndex(firstEnabledIndex(items))
    }
    if (openSubmenuIndex !== null) {
      const openEntry = items[openSubmenuIndex]
      if (!openEntry || isSeparator(openEntry) || openEntry.disabled || !hasChildren(openEntry)) {
        setOpenSubmenuIndex(null)
      }
    }
  }, [activeIndex, items, openSubmenuIndex])

  useLayoutEffect(() => {
    const menu = menuRef.current
    if (!menu) return

    const rect = menu.getBoundingClientRect()
    const viewport = currentViewport()
    let preferred = preferredPosition ?? { left: rect.left, top: rect.top }

    if (parentItem) {
      const parentRect = parentItem.getBoundingClientRect()
      preferred = preferredSubmenuPosition(
        parentRect,
        rect.width,
        viewport,
        VIEWPORT_MARGIN,
        SUBMENU_OVERLAP,
      )
    }

    const next = clampContextMenuPosition(
      preferred,
      { width: rect.width, height: rect.height },
      viewport,
    )
    setPosition((current) => current.left === next.left && current.top === next.top ? current : next)
  }, [menuRef, parentItem, preferredPosition])

  const focusItem = useCallback((index: number) => {
    if (index < 0) return
    setActiveIndex(index)
    window.requestAnimationFrame(() => focusElement(itemRefs.current[index]))
  }, [])

  useLayoutEffect(() => {
    if (autoFocus) focusItem(firstEnabledIndex(items))
  }, [autoFocus, focusItem, items])

  const openChild = useCallback((index: number, shouldFocus: boolean) => {
    const entry = items[index]
    if (!entry || isSeparator(entry) || entry.disabled || !hasChildren(entry)) return false
    setActiveIndex(index)
    setFocusSubmenu(shouldFocus)
    setOpenSubmenuIndex(index)
    return true
  }, [items])

  const activateItem = useCallback((index: number) => {
    const entry = items[index]
    if (!entry || isSeparator(entry) || entry.disabled) return
    if (hasChildren(entry)) {
      openChild(index, true)
      return
    }

    onCloseAll()
    void entry.onSelect?.()
  }, [items, onCloseAll, openChild])

  const handleKeyDown = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    switch (event.key) {
      case 'ArrowDown':
        event.preventDefault()
        event.stopPropagation()
        focusItem(adjacentEnabledIndex(items, activeIndex, 1))
        break
      case 'ArrowUp':
        event.preventDefault()
        event.stopPropagation()
        focusItem(adjacentEnabledIndex(items, activeIndex, -1))
        break
      case 'Home':
        event.preventDefault()
        event.stopPropagation()
        focusItem(firstEnabledIndex(items))
        break
      case 'End':
        event.preventDefault()
        event.stopPropagation()
        focusItem(lastEnabledIndex(items))
        break
      case 'ArrowRight':
        event.preventDefault()
        event.stopPropagation()
        openChild(activeIndex, true)
        break
      case 'ArrowLeft':
        event.preventDefault()
        event.stopPropagation()
        if (onCloseSubmenu) {
          onCloseSubmenu()
          onReturnFocus?.()
        } else if (openSubmenuIndex !== null) {
          setOpenSubmenuIndex(null)
          focusItem(activeIndex)
        }
        break
      case 'Enter':
      case ' ':
        event.preventDefault()
        event.stopPropagation()
        activateItem(activeIndex)
        break
      case 'Escape':
      case 'Tab':
        event.preventDefault()
        event.stopPropagation()
        onCloseAll()
        break
      default:
        break
    }
  }

  return (
    <div
      ref={menuRef}
      className="context-menu"
      role="menu"
      aria-label={ariaLabel}
      data-context-menu-depth={depth}
      style={{ left: position.left, top: position.top }}
      onKeyDown={handleKeyDown}
    >
      {items.map((entry, index) => {
        if (isSeparator(entry)) {
          return <div key={entry.id ?? `separator-${index}`} className="context-menu-separator" role="separator" />
        }

        const submenuOpen = openSubmenuIndex === index && hasChildren(entry)
        const itemRole = entry.checked === undefined ? 'menuitem' : 'menuitemcheckbox'
        return (
          <div
            key={entry.id}
            className="context-menu-entry"
            onMouseEnter={() => {
              setActiveIndex(index)
              setFocusSubmenu(false)
              setOpenSubmenuIndex(hasChildren(entry) && !entry.disabled ? index : null)
            }}
          >
            <button
              ref={(element) => { itemRefs.current[index] = element }}
              type="button"
              role={itemRole}
              aria-label={entry.ariaLabel}
              aria-checked={entry.checked === undefined ? undefined : entry.checked}
              aria-disabled={entry.disabled || undefined}
              aria-haspopup={hasChildren(entry) ? 'menu' : undefined}
              aria-expanded={hasChildren(entry) ? submenuOpen : undefined}
              className={`context-menu-item${activeIndex === index ? ' active' : ''}${entry.danger ? ' danger' : ''}`}
              disabled={entry.disabled}
              tabIndex={activeIndex === index && !entry.disabled ? 0 : -1}
              title={entry.disabled ? entry.disabledReason : undefined}
              onFocus={() => setActiveIndex(index)}
              onClick={() => activateItem(index)}
            >
              <span className="context-menu-check" aria-hidden="true">{entry.checked ? '✓' : ''}</span>
              <span className="context-menu-icon" aria-hidden="true">{entry.icon}</span>
              <span className="context-menu-label">{entry.label}</span>
              <span className="context-menu-shortcut" aria-hidden="true">{entry.shortcut}</span>
              <span className="context-menu-submenu-arrow" aria-hidden="true">{hasChildren(entry) ? '›' : ''}</span>
            </button>
            {submenuOpen && entry.children ? (
              <MenuLevel
                items={entry.children}
                ariaLabel={`${typeof entry.label === 'string' ? entry.label : ariaLabel} submenu`}
                onCloseAll={onCloseAll}
                parentItem={itemRefs.current[index]}
                onCloseSubmenu={() => setOpenSubmenuIndex(null)}
                onReturnFocus={() => focusItem(index)}
                autoFocus={focusSubmenu}
                depth={depth + 1}
              />
            ) : null}
          </div>
        )
      })}
    </div>
  )
}

export function ContextMenu({ request, onClose }: ContextMenuProps) {
  const rootRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!request) return undefined

    const closeForOutsidePointer = (event: PointerEvent) => {
      if (!rootRef.current?.contains(event.target as Node)) onClose()
    }
    const closeForEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') onClose()
    }

    document.addEventListener('pointerdown', closeForOutsidePointer, true)
    document.addEventListener('keydown', closeForEscape)
    window.addEventListener('blur', onClose)
    window.addEventListener('resize', onClose)
    window.visualViewport?.addEventListener('resize', onClose)

    return () => {
      document.removeEventListener('pointerdown', closeForOutsidePointer, true)
      document.removeEventListener('keydown', closeForEscape)
      window.removeEventListener('blur', onClose)
      window.removeEventListener('resize', onClose)
      window.visualViewport?.removeEventListener('resize', onClose)
    }
  }, [onClose, request])

  if (!request || typeof document === 'undefined') return null

  return createPortal(
    <MenuLevel
      key={`${request.x}:${request.y}`}
      items={request.items}
      ariaLabel={request.ariaLabel ?? 'Context menu'}
      onCloseAll={onClose}
      autoFocus
      rootRef={rootRef}
      preferredPosition={{ left: request.x, top: request.y }}
    />,
    document.body,
  )
}
