export type EditorErrorEvent =
  | { type: 'snapshot' }
  | { type: 'commandError'; message: string }
  | { type: 'reconnectSucceeded' }
  | { type: 'dismissed' }

/** Ordinary project snapshots are data updates, not error acknowledgements. */
export function reduceEditorError(current: string | undefined, event: EditorErrorEvent): string | undefined {
  switch (event.type) {
    case 'snapshot': return current
    case 'commandError': return event.message
    case 'reconnectSucceeded':
    case 'dismissed': return undefined
  }
}
