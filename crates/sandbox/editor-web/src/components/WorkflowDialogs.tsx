import { useEffect, useMemo, useState, type ReactNode } from 'react'
import type { AssetKind, AssetSnapshot, ProjectSnapshot } from '../bridge/protocol'
import type { EditorController } from '../state/useEditorState'
import { engineBridge } from '../bridge/engineBridge'

export type AssetWorkflowTarget = Pick<AssetSnapshot, 'id' | 'name' | 'path' | 'kind'>

export type WorkflowRequest =
  | { kind: 'newScene' }
  | { kind: 'sceneManager' }
  | { kind: 'saveSceneAs' }
  | { kind: 'duplicateScene'; sceneId: string }
  | { kind: 'renameScene'; sceneId: string }
  | { kind: 'deleteScene'; sceneId: string }
  | { kind: 'openProject' }
  | { kind: 'createProject' }
  | { kind: 'importAsset'; folder?: string }
  | { kind: 'createFolder'; folder?: string }
  | { kind: 'renameFolder'; folder: string }
  | { kind: 'deleteFolder'; folder: string }
  | { kind: 'createMaterial'; folder?: string }
  | { kind: 'createScript' }
  | { kind: 'createPrefab'; folder?: string }
  | { kind: 'moveAsset'; asset: AssetWorkflowTarget }
  | { kind: 'deleteAsset'; asset: AssetWorkflowTarget }

export const WORKFLOW_DIALOG_EVENT = 'engine-editor-open-workflow'

export function openWorkflowDialog(request: WorkflowRequest): void {
  window.dispatchEvent(new CustomEvent<WorkflowRequest>(WORKFLOW_DIALOG_EVENT, { detail: request }))
}

function sceneIdError(value: string): string | undefined {
  const id = value.trim()
  if (!/^[A-Za-z0-9_.-]{1,128}$/.test(id) || id === '.' || id === '..') return 'Use 1–128 ASCII letters, digits, dots, hyphens, or underscores.'
  const stem = id.split('.')[0]?.toUpperCase()
  if (stem && (/^(CON|PRN|AUX|NUL)$/.test(stem) || /^(COM|LPT)[1-9]$/.test(stem))) return 'This name is reserved by Windows.'
  return undefined
}

function relativePathError(value: string, allowEmpty = false): string | undefined {
  const path = value.trim().replaceAll('\\', '/')
  if (!path) return allowEmpty ? undefined : 'A project-relative path is required.'
  if (path.startsWith('/') || /^[A-Za-z]:/.test(path)) return 'Use a relative path, not an absolute path.'
  const parts = path.split('/')
  if (parts.some((part) => !part || part === '.' || part === '..')) return 'Path segments cannot be empty, “.”, or “..”.'
  if (/[<>:"|?*\x00-\x1f]/.test(path)) return 'The path contains characters that are not portable.'
  if (parts.some((part) => part.endsWith('.') || part.endsWith(' '))) return 'Path segments cannot end in a dot or space.'
  if (parts.some((part) => /^(CON|PRN|AUX|NUL|COM[1-9]|LPT[1-9])(?:\.|$)/i.test(part))) return 'The path contains a Windows-reserved name.'
  return undefined
}

function assetIdError(value: string): string | undefined {
  const id = value.trim()
  if (!/^[A-Za-z0-9][A-Za-z0-9_.-]{0,127}$/.test(id)) return 'Use 1–128 ASCII letters, digits, dots, hyphens, or underscores, starting with a letter or digit.'
  const stem = id.split('.')[0]?.toUpperCase()
  if (stem && (/^(CON|PRN|AUX|NUL)$/.test(stem) || /^(COM|LPT)[1-9]$/.test(stem))) return 'This asset ID is reserved by Windows.'
  return undefined
}

interface DialogFrameProps {
  title: string
  busy: boolean
  error?: string
  submitLabel?: string
  submitDisabled?: boolean
  onClose(): void
  onSubmit?: () => void
  children: ReactNode
}

function DialogFrame({ title, busy, error, submitLabel, submitDisabled, onClose, onSubmit, children }: DialogFrameProps) {
  useEffect(() => {
    const closeOnEscape = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !busy) { event.preventDefault(); onClose() }
    }
    window.addEventListener('keydown', closeOnEscape)
    return () => window.removeEventListener('keydown', closeOnEscape)
  }, [busy, onClose])

  const content = <>
    <header className="workflow-dialog-header"><h2>{title}</h2><button type="button" aria-label="Close" disabled={busy} onClick={onClose}>×</button></header>
    <div className="workflow-dialog-body">{children}{error && <div className="workflow-error" role="alert">{error}</div>}</div>
    <footer className="workflow-dialog-footer"><button type="button" disabled={busy} onClick={onClose}>Cancel</button>{submitLabel && <button className="primary" type="submit" disabled={busy || submitDisabled}>{busy ? 'Working…' : submitLabel}</button>}</footer>
  </>

  return <div className="modal-backdrop workflow-backdrop" onPointerDown={() => !busy && onClose()}><section className="workflow-dialog" role="dialog" aria-modal="true" aria-label={title} onPointerDown={(event) => event.stopPropagation()}>{onSubmit
    ? <form onSubmit={(event) => { event.preventDefault(); onSubmit() }}>{content}</form>
    : content}</section></div>
}

function useOperation(onClose: () => void) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string>()
  const run = async (operation: () => Promise<unknown | undefined>, close = true) => {
    setBusy(true); setError(undefined)
    try {
      const result = await operation()
      if (result === undefined) {
        setError('The operation failed. Review the editor error banner or Console for the exact reason.')
        return false
      }
      if (typeof result === 'object' && result !== null && 'accepted' in result && result.accepted === false) {
        setError('The editor rejected this operation. Review the editor error banner or Console for the exact reason.')
        return false
      }
      const jobId = typeof result === 'object' && result !== null && 'jobId' in result && typeof result.jobId === 'number' ? result.jobId : undefined
      if (jobId !== undefined) {
        const deadline = Date.now() + 5 * 60_000
        while (Date.now() < deadline) {
          await new Promise((resolve) => window.setTimeout(resolve, 80))
          const snapshot: ProjectSnapshot = await engineBridge.invoke('editor.getSnapshot', {})
        const status = snapshot.backgroundOperations?.find((operation) => operation.id === jobId)
          ?? (snapshot.backgroundOperation?.id === jobId ? snapshot.backgroundOperation : undefined)
        if (!status || status.state === 'running') continue
        if (status.state === 'failed') {
          setError(status.error ?? `${status.label} failed.`)
          return false
        }
        if (status.state === 'committedWithWarning') {
          if (close) onClose()
          return true
        }
        if (close) onClose()
          return true
        }
        setError('The editor operation did not report completion within five minutes.')
        return false
      }
      if (close) onClose()
      return true
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason))
      return false
    } finally {
      setBusy(false)
    }
  }
  return { busy, error, setError, run }
}

function Field({ label, hint, error, children }: { label: string; hint?: string; error?: string; children: ReactNode }) {
  return <label className="workflow-field"><span>{label}</span>{children}{hint && <small>{hint}</small>}{error && <small className="field-error">{error}</small>}</label>
}

function NewSceneDialog({ controller, onClose }: { controller: EditorController; onClose(): void }) {
  const [sceneId, setSceneId] = useState('new-scene')
  const [folder, setFolder] = useState('')
  const operation = useOperation(onClose)
  const idError = sceneIdError(sceneId)
  const folderError = relativePathError(folder, true)
  return <DialogFrame title="Create Scene" busy={operation.busy} error={operation.error} submitLabel="Create and Open" submitDisabled={Boolean(idError || folderError)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('document.create', { sceneId: sceneId.trim(), folder: folder.trim().replaceAll('\\', '/') }))}>
    <Field label="Scene ID" error={idError} hint="Stable catalog ID and default filename."><input autoFocus value={sceneId} onChange={(event) => setSceneId(event.target.value)} /></Field>
    <Field label="Scene subfolder" error={folderError} hint="Optional path below assets/scenes."><input value={folder} placeholder="levels" onChange={(event) => setFolder(event.target.value)} /></Field>
  </DialogFrame>
}

function SaveSceneAsDialog({ controller, project, onClose }: { controller: EditorController; project: ProjectSnapshot; onClose(): void }) {
  const [sceneId, setSceneId] = useState(`${project.document.currentSceneId}-copy`)
  const operation = useOperation(onClose)
  const error = sceneIdError(sceneId) ?? (project.document.scenes.some((scene) => scene.id.toLocaleLowerCase() === sceneId.trim().toLocaleLowerCase()) ? 'A scene with this ID already exists.' : undefined)
  return <DialogFrame title="Save Scene As" busy={operation.busy} error={operation.error} submitLabel="Save As" submitDisabled={Boolean(error)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('document.saveAs', { sceneId: sceneId.trim() }))}><Field label="New scene ID" error={error}><input autoFocus value={sceneId} onChange={(event) => setSceneId(event.target.value)} /></Field></DialogFrame>
}

function SceneIdentityDialog({ request, controller, project, onClose }: { request: Extract<WorkflowRequest, { kind: 'duplicateScene' | 'renameScene' }>; controller: EditorController; project: ProjectSnapshot; onClose(): void }) {
  const duplicate = request.kind === 'duplicateScene'
  const [newId, setNewId] = useState(duplicate ? `${request.sceneId}-copy` : request.sceneId)
  const operation = useOperation(onClose)
  const validation = sceneIdError(newId) ?? (newId.trim().toLocaleLowerCase() !== request.sceneId.toLocaleLowerCase() && project.document.scenes.some((scene) => scene.id.toLocaleLowerCase() === newId.trim().toLocaleLowerCase()) ? 'A scene with this ID already exists.' : undefined)
  const unchanged = !duplicate && newId.trim() === request.sceneId
  return <DialogFrame title={duplicate ? `Duplicate “${request.sceneId}”` : `Rename “${request.sceneId}”`} busy={operation.busy} error={operation.error} submitLabel={duplicate ? 'Duplicate and Open' : 'Rename'} submitDisabled={Boolean(validation || unchanged)} onClose={onClose} onSubmit={() => void operation.run(() => duplicate
    ? controller.invoke('document.duplicate', { sourceId: request.sceneId, newId: newId.trim() })
    : controller.invoke('document.rename', { oldId: request.sceneId, newId: newId.trim() }))}><Field label="Scene ID" error={validation}><input autoFocus value={newId} onChange={(event) => setNewId(event.target.value)} /></Field></DialogFrame>
}

function DeleteSceneDialog({ sceneId, controller, project, onClose }: { sceneId: string; controller: EditorController; project: ProjectSnapshot; onClose(): void }) {
  const scene = project.document.scenes.find((entry) => entry.id === sceneId)
  const alternatives = project.document.scenes.filter((entry) => entry.id !== sceneId)
  const [replacement, setReplacement] = useState(alternatives[0]?.id ?? '')
  const operation = useOperation(onClose)
  const blocked = alternatives.length === 0
  const replacementRequired = Boolean(scene?.startup && !replacement)
  return <DialogFrame title={`Delete “${sceneId}”`} busy={operation.busy} error={operation.error} submitLabel="Move to Trash" submitDisabled={blocked || replacementRequired} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('document.delete', { sceneId, replacementStartup: scene?.startup ? replacement : undefined }))}>
    <p>This removes the scene from the project catalog and moves its files to the recoverable project trash.</p>
    {blocked && <div className="workflow-error">The final project scene cannot be deleted.</div>}
    {scene?.startup && <Field label="Replacement startup scene"><select value={replacement} onChange={(event) => setReplacement(event.target.value)}><option value="" disabled>Select a scene</option>{alternatives.map((entry) => <option key={entry.id} value={entry.id}>{entry.id}</option>)}</select></Field>}
  </DialogFrame>
}

function SceneManagerDialog({ controller, project, onClose, onRequest }: { controller: EditorController; project: ProjectSnapshot; onClose(): void; onRequest(request: WorkflowRequest): void }) {
  const operation = useOperation(onClose)
  return <DialogFrame title="Project Scenes" busy={operation.busy} error={operation.error} onClose={onClose}>
    <div className="scene-document-list">{project.document.scenes.map((scene) => <div className={`scene-document-row ${scene.current ? 'current' : ''}`} key={scene.id}><div><strong>{scene.id}</strong><small>{scene.path}</small><span>{scene.current ? 'Open' : ''}{scene.startup ? `${scene.current ? ' · ' : ''}Startup` : ''}</span></div><div className="scene-document-actions"><button type="button" disabled={operation.busy || scene.current} onClick={() => void operation.run(() => controller.invoke('document.open', { sceneId: scene.id }))}>Open</button><button type="button" disabled={operation.busy} onClick={() => onRequest({ kind: 'duplicateScene', sceneId: scene.id })}>Duplicate</button><button type="button" disabled={operation.busy} onClick={() => onRequest({ kind: 'renameScene', sceneId: scene.id })}>Rename</button><button type="button" disabled={operation.busy || scene.startup} onClick={() => void operation.run(() => controller.invoke('document.setStartup', { sceneId: scene.id }), false)}>Set Startup</button><button className="danger" type="button" disabled={operation.busy} onClick={() => onRequest({ kind: 'deleteScene', sceneId: scene.id })}>Delete</button></div></div>)}</div>
    <button type="button" onClick={() => onRequest({ kind: 'newScene' })}>Create Scene…</button>
  </DialogFrame>
}

function ProjectDialog({ mode, controller, onClose }: { mode: 'open' | 'create'; controller: EditorController; onClose(): void }) {
  const create = mode === 'create'
  const [path, setPath] = useState('')
  const [name, setName] = useState('')
  const [withCsharp, setWithCsharp] = useState(true)
  const operation = useOperation(onClose)
  const pathError = path.trim() ? undefined : create ? 'Choose the directory that will contain the new project.' : 'Enter a project directory or project manifest path.'
  return <DialogFrame title={create ? 'Create Project' : 'Open Project'} busy={operation.busy} error={operation.error} submitLabel={create ? 'Create in New Window' : 'Open in New Window'} submitDisabled={Boolean(pathError)} onClose={onClose} onSubmit={() => void operation.run(() => create
    ? controller.invoke('project.create', { root: path.trim(), name: name.trim() || undefined, withCsharp })
    : controller.invoke('project.open', { path: path.trim() }))}>
    <Field label={create ? 'Project directory' : 'Project path'} error={pathError} hint="Absolute paths are recommended."><input autoFocus value={path} placeholder="E:\\projects\\my-game" onChange={(event) => setPath(event.target.value)} /></Field>
    {create && <><Field label="Project name" hint="Optional; the directory name is used when empty."><input value={name} onChange={(event) => setName(event.target.value)} /></Field><label className="workflow-check"><input type="checkbox" checked={withCsharp} onChange={(event) => setWithCsharp(event.target.checked)} /><span>Create C# scripting project</span></label></>}
  </DialogFrame>
}

const ASSET_TYPES = ['Mesh', 'Texture', 'Material', 'Audio', 'Animation', 'Skeleton', 'NavMesh', 'Prefab'] as const

function ImportAssetDialog({ initialFolder, controller, onClose }: { initialFolder?: string; controller: EditorController; onClose(): void }) {
  const [source, setSource] = useState('')
  const [assetId, setAssetId] = useState('')
  const [assetType, setAssetType] = useState<(typeof ASSET_TYPES)[number]>('Texture')
  const [folder, setFolder] = useState(initialFolder ?? '')
  const operation = useOperation(onClose)
  const sourceError = source.trim() ? undefined : 'Source file path is required.'
  const idError = assetIdError(assetId)
  const folderError = relativePathError(folder, true)
  return <DialogFrame title="Import Asset" busy={operation.busy} error={operation.error} submitLabel="Import and Cook" submitDisabled={Boolean(sourceError || idError || folderError)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.import', { source: source.trim(), assetId: assetId.trim(), assetType, folder: folder.trim().replaceAll('\\', '/') }))}>
    <Field label="Source file" error={sourceError} hint="Absolute source path or a path readable by the editor process."><input autoFocus value={source} onChange={(event) => setSource(event.target.value)} /></Field>
    <Field label="Stable asset ID" error={idError} hint="For example texture-player-albedo."><input value={assetId} onChange={(event) => setAssetId(event.target.value)} /></Field>
    <Field label="Destination folder" error={folderError} hint="Relative to assets/source; the folder must already exist."><input value={folder} onChange={(event) => setFolder(event.target.value)} /></Field>
    <Field label="Asset type"><select value={assetType} onChange={(event) => setAssetType(event.target.value as (typeof ASSET_TYPES)[number])}>{ASSET_TYPES.map((type) => <option key={type}>{type}</option>)}</select></Field>
  </DialogFrame>
}

function CreateFolderDialog({ initialFolder, controller, onClose }: { initialFolder?: string; controller: EditorController; onClose(): void }) {
  const [parent, setParent] = useState(initialFolder ?? '')
  const [name, setName] = useState('new-folder')
  const operation = useOperation(onClose)
  const target = [parent.trim().replaceAll('\\', '/').replace(/\/$/, ''), name.trim()].filter(Boolean).join('/')
  const error = !name.trim() || name.includes('/') || name.includes('\\') ? 'Folder name must be one non-empty path segment.' : relativePathError(target)
  return <DialogFrame title="Create Asset Folder" busy={operation.busy} error={operation.error} submitLabel="Create Folder" submitDisabled={Boolean(error)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.createFolder', { folder: target }))}><Field label="Parent folder" hint="Relative to assets/source."><input value={parent} onChange={(event) => setParent(event.target.value)} /></Field><Field label="Folder name" error={error}><input autoFocus value={name} onChange={(event) => setName(event.target.value)} /></Field></DialogFrame>
}

function RenameFolderDialog({ folder, controller, onClose }: { folder: string; controller: EditorController; onClose(): void }) {
  const normalized = folder.replace(/^\/+|\/+$/g, '')
  const parts = normalized.split('/')
  const currentName = parts.pop() ?? ''
  const parent = parts.join('/')
  const [name, setName] = useState(currentName)
  const operation = useOperation(onClose)
  const target = [parent, name.trim()].filter(Boolean).join('/')
  const error = !name.trim() || name.includes('/') || name.includes('\\')
    ? 'Folder name must be one non-empty path segment.'
    : relativePathError(target)
  return <DialogFrame title={`Rename folder "${currentName}"`} busy={operation.busy} error={operation.error} submitLabel="Rename Folder" submitDisabled={Boolean(error || name.trim() === currentName)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.renameFolder', { folder: normalized, newFolder: target }))}>
    <Field label="Folder name" error={error} hint={parent ? `Parent: ${parent}` : 'Parent: assets/source'}><input autoFocus value={name} onChange={(event) => setName(event.target.value)} /></Field>
  </DialogFrame>
}

function DeleteFolderDialog({ folder, controller, onClose }: { folder: string; controller: EditorController; onClose(): void }) {
  const normalized = folder.replace(/^\/+|\/+$/g, '')
  const [confirmed, setConfirmed] = useState(false)
  const operation = useOperation(onClose)
  return <DialogFrame title={`Delete folder "${normalized}"`} busy={operation.busy} error={operation.error} submitLabel="Delete Empty Folder" submitDisabled={!confirmed} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.deleteFolder', { folder: normalized }))}>
    <p>Only empty folders can be removed. Move or delete contained assets and subfolders through their own dependency-aware operations first.</p>
    <label className="workflow-check"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} /><span>Delete this empty folder.</span></label>
  </DialogFrame>
}

function CreateMaterialDialog({ initialFolder, controller, onClose }: { initialFolder?: string; controller: EditorController; onClose(): void }) {
  const [folder, setFolder] = useState(initialFolder ?? '')
  const [name, setName] = useState('New Material')
  const operation = useOperation(onClose)
  const folderError = relativePathError(folder, true)
  const nameError = name.trim() ? undefined : 'Material name is required.'
  return <DialogFrame title="Create Material" busy={operation.busy} error={operation.error} submitLabel="Create and Cook" submitDisabled={Boolean(folderError || nameError)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.createMaterial', { folder: folder.trim().replaceAll('\\', '/'), name: name.trim() }))}><Field label="Asset folder" error={folderError} hint="Relative to assets/source; the folder must already exist."><input value={folder} onChange={(event) => setFolder(event.target.value)} /></Field><Field label="Material name" error={nameError}><input autoFocus value={name} onChange={(event) => setName(event.target.value)} /></Field></DialogFrame>
}

function CreateScriptDialog({ controller, onClose }: { controller: EditorController; onClose(): void }) {
  const [folder, setFolder] = useState('')
  const [className, setClassName] = useState('NewBehaviour')
  const operation = useOperation(onClose)
  const folderError = relativePathError(folder, true)
  const classError = /^[A-Za-z_][A-Za-z0-9_]*$/.test(className.trim()) ? undefined : 'Enter a valid C# class identifier.'
  return <DialogFrame title="Create C# Script" busy={operation.busy} error={operation.error} submitLabel="Create Script" submitDisabled={Boolean(folderError || classError)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('script.create', { className: className.trim(), folder: folder.trim().replaceAll('\\', '/') }))}><p>The current project must have a C# script project configured.</p><Field label="Class name" error={classError}><input autoFocus value={className} onChange={(event) => setClassName(event.target.value)} /></Field><Field label="Script subfolder" error={folderError} hint="Optional path below the C# project source directory."><input value={folder} onChange={(event) => setFolder(event.target.value)} /></Field></DialogFrame>
}

function CreatePrefabDialog({ initialFolder, controller, project, onClose }: { initialFolder?: string; controller: EditorController; project: ProjectSnapshot; onClose(): void }) {
  const base = project.selection.displayName?.trim().toLocaleLowerCase().replace(/[^a-z0-9_-]+/g, '-') || 'new-prefab'
  const folder = initialFolder?.replace(/^assets\/?/i, '').replace(/\/$/, '') ?? ''
  const [assetId, setAssetId] = useState(`prefab-${base}`)
  const [sourcePath, setSourcePath] = useState([folder, `${base}.prefab.ron`].filter(Boolean).join('/'))
  const [manifestName, setManifestName] = useState('prefabs.manifest')
  const operation = useOperation(onClose)
  const idError = assetIdError(assetId)
  const sourceError = relativePathError(sourcePath) ?? (sourcePath.toLocaleLowerCase().endsWith('.prefab.ron') ? undefined : 'Prefab source must end in .prefab.ron.')
  const manifestError = relativePathError(manifestName) ?? (manifestName.includes('/') || manifestName.includes('\\') ? 'Manifest must be a top-level file in assets/source.' : !manifestName.toLocaleLowerCase().endsWith('.manifest') ? 'Manifest filename must end in .manifest.' : undefined)
  const selectionError = project.selection.activeEntityId ? undefined : 'Select a scene entity hierarchy first.'
  return <DialogFrame title="Create Prefab from Selection" busy={operation.busy} error={operation.error} submitLabel="Create Prefab" submitDisabled={Boolean(idError || sourceError || manifestError || selectionError)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.createPrefab', { assetId: assetId.trim(), relativeSourcePath: sourcePath.trim().replaceAll('\\', '/'), manifestName: manifestName.trim() }))}>
    {selectionError && <div className="workflow-error">{selectionError}</div>}<Field label="Stable asset ID" error={idError}><input autoFocus value={assetId} onChange={(event) => setAssetId(event.target.value)} /></Field><Field label="Prefab source path" error={sourceError} hint="Relative to assets/source."><input value={sourcePath} onChange={(event) => setSourcePath(event.target.value)} /></Field><Field label="Source manifest" error={manifestError} hint="Top-level manifest inside assets/source."><input value={manifestName} onChange={(event) => setManifestName(event.target.value)} /></Field>
  </DialogFrame>
}

function MoveAssetDialog({ asset, controller, onClose }: { asset: AssetWorkflowTarget; controller: EditorController; onClose(): void }) {
  const initialPath = asset.path.replace(/^assets\/?/i, '')
  const [path, setPath] = useState(initialPath)
  const operation = useOperation(onClose)
  const error = relativePathError(path)
  return <DialogFrame title={`Move “${asset.name}”`} busy={operation.busy} error={operation.error} submitLabel="Move Asset" submitDisabled={Boolean(error || path.trim().replaceAll('\\', '/') === initialPath)} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.move', { assetId: asset.id, newSourcePath: path.trim().replaceAll('\\', '/') }))}><Field label="New source path" error={error} hint="Relative to assets/source, including the filename."><input autoFocus value={path} onChange={(event) => setPath(event.target.value)} /></Field></DialogFrame>
}

function DeleteAssetDialog({ asset, controller, onClose }: { asset: AssetWorkflowTarget; controller: EditorController; onClose(): void }) {
  const [confirmed, setConfirmed] = useState(false)
  const operation = useOperation(onClose)
  return <DialogFrame title={`Delete “${asset.name}”`} busy={operation.busy} error={operation.error} submitLabel="Move to Trash" submitDisabled={!confirmed} onClose={onClose} onSubmit={() => void operation.run(() => controller.invoke('assets.delete', { assetId: asset.id }))}><p>The source, cooked payload, and recovery metadata will be moved to the project trash. Referenced assets cannot be deleted.</p><label className="workflow-check"><input type="checkbox" checked={confirmed} onChange={(event) => setConfirmed(event.target.checked)} /><span>I understand this asset will be removed from the manifest.</span></label></DialogFrame>
}

export function WorkflowDialogs({ request, controller, onRequest, onClose }: { request?: WorkflowRequest; controller: EditorController; onRequest(request: WorkflowRequest): void; onClose(): void }) {
  const project = controller.state.project
  const key = useMemo(() => request ? JSON.stringify(request) : '', [request])
  if (!request) return null
  const shared = { controller, onClose }
  let dialog: ReactNode
  switch (request.kind) {
    case 'newScene': dialog = <NewSceneDialog {...shared} />; break
    case 'sceneManager': dialog = <SceneManagerDialog {...shared} project={project} onRequest={onRequest} />; break
    case 'saveSceneAs': dialog = <SaveSceneAsDialog {...shared} project={project} />; break
    case 'duplicateScene': case 'renameScene': dialog = <SceneIdentityDialog {...shared} request={request} project={project} />; break
    case 'deleteScene': dialog = <DeleteSceneDialog {...shared} sceneId={request.sceneId} project={project} />; break
    case 'openProject': dialog = <ProjectDialog {...shared} mode="open" />; break
    case 'createProject': dialog = <ProjectDialog {...shared} mode="create" />; break
    case 'importAsset': dialog = <ImportAssetDialog {...shared} initialFolder={request.folder} />; break
    case 'createFolder': dialog = <CreateFolderDialog {...shared} initialFolder={request.folder} />; break
    case 'renameFolder': dialog = <RenameFolderDialog {...shared} folder={request.folder} />; break
    case 'deleteFolder': dialog = <DeleteFolderDialog {...shared} folder={request.folder} />; break
    case 'createMaterial': dialog = <CreateMaterialDialog {...shared} initialFolder={request.folder} />; break
    case 'createScript': dialog = <CreateScriptDialog {...shared} />; break
    case 'createPrefab': dialog = <CreatePrefabDialog {...shared} project={project} initialFolder={request.folder} />; break
    case 'moveAsset': dialog = <MoveAssetDialog {...shared} asset={request.asset} />; break
    case 'deleteAsset': dialog = <DeleteAssetDialog {...shared} asset={request.asset} />; break
  }
  return <div key={key}>{dialog}</div>
}

export function sceneForAsset(project: ProjectSnapshot, asset: { id: string; path: string; kind: AssetKind }) {
  if (asset.kind !== 'scene') return undefined
  const normalized = asset.path.replaceAll('\\', '/').toLocaleLowerCase()
  return project.document.scenes.find((scene) => scene.id === asset.id || scene.path.replaceAll('\\', '/').toLocaleLowerCase().endsWith(normalized))
}
