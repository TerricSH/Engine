import type { ReactNode, SVGProps } from 'react'

export type IconName =
  | 'add'
  | 'animation'
  | 'asset'
  | 'back'
  | 'build'
  | 'camera'
  | 'chevron'
  | 'close'
  | 'collapse'
  | 'console'
  | 'cube'
  | 'error'
  | 'eye'
  | 'filter'
  | 'folder'
  | 'forward'
  | 'game'
  | 'hand'
  | 'hierarchy'
  | 'info'
  | 'inspector'
  | 'layout'
  | 'link'
  | 'lock'
  | 'maximize'
  | 'menu'
  | 'move'
  | 'pause'
  | 'play'
  | 'profiler'
  | 'rect'
  | 'refresh'
  | 'rotate'
  | 'scale'
  | 'scene'
  | 'search'
  | 'settings'
  | 'sphere'
  | 'snap'
  | 'step'
  | 'warning'

const paths: Record<IconName, ReactNode> = {
  add: <path d="M12 5v14M5 12h14" />,
  animation: <><path d="M4 6h16v12H4z" /><path d="m10 9 5 3-5 3z" /></>,
  asset: <><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z" /><path d="m4.5 7.7 7.5 4.2 7.5-4.2M12 12v9" /></>,
  back: <path d="m15 18-6-6 6-6" />,
  build: <><path d="m14 5 5 5-9 9H5v-5z" /><path d="m12 7 5 5" /></>,
  camera: <><path d="M3 7h4l2-2h6l2 2h4v12H3z" /><circle cx="12" cy="13" r="4" /></>,
  chevron: <path d="m9 18 6-6-6-6" />,
  close: <path d="m6 6 12 12M18 6 6 18" />,
  collapse: <path d="m8 10 4 4 4-4" />,
  console: <><path d="M4 5h16v14H4z" /><path d="m7 9 3 3-3 3M12 15h5" /></>,
  cube: <><path d="m12 3 8 4.5v9L12 21l-8-4.5v-9z" /><path d="m4 7.5 8 4.5 8-4.5M12 12v9" /></>,
  error: <><circle cx="12" cy="12" r="9" /><path d="m9 9 6 6m0-6-6 6" /></>,
  eye: <><path d="M2.5 12s3.5-6 9.5-6 9.5 6 9.5 6-3.5 6-9.5 6-9.5-6-9.5-6" /><circle cx="12" cy="12" r="2.5" /></>,
  filter: <path d="M4 5h16l-6.5 7.5V19l-3 1v-7.5z" />,
  folder: <path d="M3 6h7l2 2h9v11H3z" />,
  forward: <path d="m9 18 6-6-6-6" />,
  game: <><path d="M7 9h10l3 8-3 2-3-3h-4l-3 3-3-2z" /><path d="M8 12v4m-2-2h4M16 13h.01M18 15h.01" /></>,
  hand: <path d="M7 11V7a1.5 1.5 0 0 1 3 0v3-5a1.5 1.5 0 0 1 3 0v5-4a1.5 1.5 0 0 1 3 0v5-2a1.5 1.5 0 0 1 3 0v5c0 4-2.5 7-7 7-3 0-5-1.5-7-5l-2-3a1.6 1.6 0 0 1 2.5-2z" />,
  hierarchy: <><circle cx="6" cy="6" r="2" /><circle cx="18" cy="8" r="2" /><circle cx="18" cy="17" r="2" /><path d="M8 6h3v11h5M11 8h5" /></>,
  info: <><circle cx="12" cy="12" r="9" /><path d="M12 11v6M12 7h.01" /></>,
  inspector: <><path d="M5 4h14v16H5z" /><path d="M8 8h8M8 12h5M8 16h7" /></>,
  layout: <><path d="M3 4h18v16H3zM8 4v16M16 4v16" /></>,
  link: <><path d="M10 13a4 4 0 0 0 5.5.2l2.3-2.3a4 4 0 0 0-5.7-5.7L10.8 6.5" /><path d="M14 11a4 4 0 0 0-5.5-.2l-2.3 2.3a4 4 0 0 0 5.7 5.7l1.3-1.3" /></>,
  lock: <><rect x="5" y="10" width="14" height="10" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" /></>,
  maximize: <><path d="M4 9V4h5M15 4h5v5M20 15v5h-5M9 20H4v-5" /></>,
  menu: <><path d="M5 7h14M5 12h14M5 17h14" /></>,
  move: <><path d="M12 2v20M2 12h20" /><path d="m9 5 3-3 3 3M19 9l3 3-3 3M9 19l3 3 3-3M5 9l-3 3 3 3" /></>,
  pause: <><path d="M8 5v14M16 5v14" /></>,
  play: <path d="m8 5 11 7-11 7z" />,
  profiler: <><path d="M4 19V9M9 19V5M14 19v-7M19 19V3" /></>,
  rect: <><path d="M5 5h14v14H5z" /><path d="M3 8h4M17 16h4" /></>,
  refresh: <><path d="M20 11a8 8 0 0 0-14-5L4 8" /><path d="M4 4v4h4M4 13a8 8 0 0 0 14 5l2-2M20 20v-4h-4" /></>,
  rotate: <><circle cx="12" cy="12" r="7" /><path d="M12 2v4m0 12v4M2 12h4m12 0h4" /></>,
  scale: <><path d="M5 19 19 5M11 5h8v8M5 11v8h8" /></>,
  scene: <><path d="M4 4h16v16H4z" /><path d="m4 16 5-5 4 4 2-2 5 5" /><circle cx="15" cy="8" r="1.5" /></>,
  search: <><circle cx="10.5" cy="10.5" r="6.5" /><path d="m16 16 5 5" /></>,
  settings: <><circle cx="12" cy="12" r="3" /><path d="M19 13.5v-3l-2-.6a7 7 0 0 0-.7-1.7l1-1.9-2.1-2.1-1.9 1a7 7 0 0 0-1.7-.7L11 2H8l-.6 2.5a7 7 0 0 0-1.7.7l-1.9-1-2.1 2.1 1 1.9A7 7 0 0 0 2 9.9L0 10.5v3l2 .6a7 7 0 0 0 .7 1.7l-1 1.9 2.1 2.1 1.9-1a7 7 0 0 0 1.7.7L8 22h3l.6-2.5a7 7 0 0 0 1.7-.7l1.9 1 2.1-2.1-1-1.9a7 7 0 0 0 .7-1.7z" /></>,
  sphere: <><circle cx="12" cy="12" r="9" /><path d="M4 9c4 2 12 2 16 0M5 16c4-2 10-2 14 0M12 3c-3 3-3 15 0 18M12 3c3 3 3 15 0 18" /></>,
  snap: <><path d="M6 4v9a6 6 0 0 0 12 0V4" /><path d="M6 8h4M14 8h4M6 4h4M14 4h4" /></>,
  step: <><path d="m7 5 9 7-9 7zM18 5v14" /></>,
  warning: <><path d="m12 3 10 18H2z" /><path d="M12 9v5M12 18h.01" /></>,
}

export function Icon({ name, ...props }: { name: IconName } & SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" {...props}>
      {paths[name]}
    </svg>
  )
}
