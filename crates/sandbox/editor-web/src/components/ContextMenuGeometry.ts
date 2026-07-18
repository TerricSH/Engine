export interface ContextMenuViewportBounds {
  left: number
  top: number
  width: number
  height: number
}

export interface ContextMenuPosition {
  left: number
  top: number
}

export function clampContextMenuPosition(
  preferred: ContextMenuPosition,
  menuSize: { width: number; height: number },
  viewport: ContextMenuViewportBounds,
  margin = 6,
): ContextMenuPosition {
  const minimumLeft = viewport.left + margin
  const minimumTop = viewport.top + margin
  const maximumLeft = Math.max(minimumLeft, viewport.left + viewport.width - menuSize.width - margin)
  const maximumTop = Math.max(minimumTop, viewport.top + viewport.height - menuSize.height - margin)

  return {
    left: Math.min(maximumLeft, Math.max(minimumLeft, preferred.left)),
    top: Math.min(maximumTop, Math.max(minimumTop, preferred.top)),
  }
}

export function preferredSubmenuPosition(
  parent: { left: number; right: number; top: number },
  submenuWidth: number,
  viewport: ContextMenuViewportBounds,
  margin = 6,
  overlap = 3,
): ContextMenuPosition {
  const right = parent.right - overlap
  const fitsToRight = right + submenuWidth + margin <= viewport.left + viewport.width
  return {
    left: fitsToRight ? right : parent.left - submenuWidth + overlap,
    top: parent.top - 3,
  }
}
