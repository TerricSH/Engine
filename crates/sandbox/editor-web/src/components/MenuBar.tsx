import { useEffect, useRef, useState } from 'react'
import { Icon } from './Icon'

export interface MenuCommand { id: string; label: string; shortcut?: string; danger?: boolean; separatorBefore?: boolean }
interface MenuDefinition { label: string; items: MenuCommand[] }
const menus: MenuDefinition[] = [
  { label: 'File', items: [
    { id: 'file.newScene', label: 'New Scene…', shortcut: 'Ctrl+N' },
    { id: 'file.openScene', label: 'Open Scene…' },
    { id: 'file.createProject', label: 'Create Project…', separatorBefore: true },
    { id: 'file.openProject', label: 'Open Project…', shortcut: 'Ctrl+O' },
    { id: 'file.saveScene', label: 'Save Scene', shortcut: 'Ctrl+S', separatorBefore: true },
    { id: 'file.saveSceneAs', label: 'Save Scene As…', shortcut: 'Ctrl+Shift+S' },
    { id: 'file.build', label: 'Build Settings…', shortcut: 'Ctrl+Shift+B', separatorBefore: true },
    { id: 'file.quit', label: 'Exit', danger: true, separatorBefore: true },
  ] },
  { label: 'Edit', items: [
    { id: 'edit.undo', label: 'Undo', shortcut: 'Ctrl+Z' }, { id: 'edit.redo', label: 'Redo', shortcut: 'Ctrl+Y' },
    { id: 'edit.cut', label: 'Cut', shortcut: 'Ctrl+X', separatorBefore: true }, { id: 'edit.copy', label: 'Copy', shortcut: 'Ctrl+C' },
    { id: 'edit.paste', label: 'Paste', shortcut: 'Ctrl+V' }, { id: 'edit.duplicate', label: 'Duplicate', shortcut: 'Ctrl+D' },
    { id: 'edit.delete', label: 'Delete', shortcut: 'Del' }, { id: 'edit.projectSettings', label: 'Project Settings…', separatorBefore: true },
  ] },
  { label: 'Assets', items: [
    { id: 'assets.createFolder', label: 'Create Folder…' }, { id: 'assets.createMaterial', label: 'Create Material…' },
    { id: 'assets.createScript', label: 'Create C# Script…' }, { id: 'assets.createPrefab', label: 'Create Prefab from Selection…' },
    { id: 'assets.import', label: 'Import New Asset…', separatorBefore: true },
    { id: 'assets.refresh', label: 'Refresh', shortcut: 'Ctrl+R' },
    { id: 'assets.reveal', label: 'Show in Explorer', separatorBefore: true },
  ] },
  { label: 'GameObject', items: [
    { id: 'gameObject.empty', label: 'Create Empty', shortcut: 'Ctrl+Shift+N' }, { id: 'gameObject.cube', label: '3D Object / Cube' },
    { id: 'gameObject.camera', label: 'Camera' }, { id: 'gameObject.light', label: 'Light' }, { id: 'gameObject.audioListener', label: 'Audio Listener' },
  ] },
  { label: 'Component', items: [
    { id: 'component.add', label: 'Add Component…', shortcut: 'Ctrl+Shift+A' },
    { id: 'component.resetTransform', label: 'Reset Transform', separatorBefore: true },
  ] },
  { label: 'Window', items: [
    { id: 'window.scene', label: 'General / Scene' }, { id: 'window.game', label: 'General / Game' }, { id: 'window.hierarchy', label: 'General / Hierarchy' },
    { id: 'window.inspector', label: 'General / Inspector' }, { id: 'window.project', label: 'General / Project' },
    { id: 'window.console', label: 'General / Console', shortcut: 'Ctrl+Shift+C' }, { id: 'window.material', label: 'Shading / Material', separatorBefore: true },
    { id: 'window.animation', label: 'Animation / Animation' },
    { id: 'window.profiler', label: 'Analysis / Profiler' }, { id: 'window.build', label: 'Build' }, { id: 'window.resetLayout', label: 'Layouts / Default', separatorBefore: true },
  ] },
]

interface MenuBarProps {
  projectName: string
  sceneName: string
  sceneDirty: boolean
  disabledCommands: ReadonlySet<string>
  onCommand(command: string): void
}

export function MenuBar({ projectName, sceneName, sceneDirty, disabledCommands, onCommand }: MenuBarProps) {
  const [openMenu, setOpenMenu] = useState<number>()
  const barRef = useRef<HTMLDivElement>(null)
  useEffect(() => {
    const close = (event: PointerEvent) => { if (!barRef.current?.contains(event.target as Node)) setOpenMenu(undefined) }
    const closeOnEscape = (event: KeyboardEvent) => { if (event.key === 'Escape') setOpenMenu(undefined) }
    window.addEventListener('pointerdown', close); window.addEventListener('keydown', closeOnEscape)
    return () => { window.removeEventListener('pointerdown', close); window.removeEventListener('keydown', closeOnEscape) }
  }, [])
  return <div className="menu-bar" ref={barRef}>
    <div className="brand-mark" aria-label="Engine Editor"><span /></div>
    <nav className="menus" aria-label="Application menu">{menus.map((menu, index) => <div className="menu-root" key={menu.label}>
      <button className={openMenu === index ? 'menu-button active' : 'menu-button'} type="button" onClick={() => setOpenMenu(openMenu === index ? undefined : index)} onPointerEnter={() => openMenu !== undefined && setOpenMenu(index)}>{menu.label}</button>
      {openMenu === index && <div className="menu-popover" role="menu">{menu.items.map((item) => <button className={`${item.separatorBefore ? 'separator ' : ''}${item.danger ? 'danger' : ''}`} disabled={disabledCommands.has(item.id)} key={item.id} role="menuitem" type="button" onClick={() => { setOpenMenu(undefined); onCommand(item.id) }}><span>{item.label}</span>{item.shortcut && <kbd>{item.shortcut}</kbd>}</button>)}</div>}
    </div>)}</nav>
    <div className="window-title" title={projectName ? `${projectName} — ${sceneName}` : 'No project loaded'}><Icon name="cube" /><span>{projectName || 'Engine Editor'}</span>{sceneName && <><span className="title-separator">/</span><span>{sceneName}{sceneDirty ? ' *' : ''}</span></>}</div>
  </div>
}
