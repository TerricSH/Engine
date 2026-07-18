export type NativeViewportKind = 'scene' | 'game'

/**
 * The Game viewport owns ordinary keyboard input while it is focused. Only
 * the explicit host-level Play/Stop key is allowed to escape to the editor;
 * command-palette routing is handled before this function is called.
 */
export function editorShortcutAllowedForViewport(
  viewport: NativeViewportKind | undefined,
  key: string,
): boolean {
  return viewport !== 'game' || key === 'F5'
}
