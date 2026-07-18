import assert from 'node:assert/strict'
import test from 'node:test'

import {
  clampContextMenuPosition,
  preferredSubmenuPosition,
} from './ContextMenuGeometry.ts'

const viewport = { left: 0, top: 0, width: 800, height: 600 }

test('clamps a root menu to every viewport edge', () => {
  assert.deepEqual(
    clampContextMenuPosition({ left: -40, top: -80 }, { width: 200, height: 300 }, viewport),
    { left: 6, top: 6 },
  )
  assert.deepEqual(
    clampContextMenuPosition({ left: 790, top: 590 }, { width: 200, height: 300 }, viewport),
    { left: 594, top: 294 },
  )
})

test('keeps oversized menus pinned to the safe viewport origin', () => {
  assert.deepEqual(
    clampContextMenuPosition({ left: 400, top: 300 }, { width: 900, height: 700 }, viewport),
    { left: 6, top: 6 },
  )
})

test('opens a submenu to the right when it fits and flips it to the left otherwise', () => {
  assert.deepEqual(
    preferredSubmenuPosition({ left: 100, right: 300, top: 40 }, 180, viewport),
    { left: 297, top: 37 },
  )
  assert.deepEqual(
    preferredSubmenuPosition({ left: 620, right: 790, top: 40 }, 180, viewport),
    { left: 443, top: 37 },
  )
})
