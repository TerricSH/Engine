import { useEffect, useState } from 'react'
import type { EditorController } from '../state/useEditorState'
import { Icon } from '../components/Icon'

type BuildOperation = 'validate' | 'cookAndCompile' | 'packageWindows'

const OPERATION_LABELS: Record<BuildOperation, string> = {
  validate: 'Validate Project',
  cookAndCompile: 'Cook & Compile',
  packageWindows: 'Package Windows',
}

export function BuildPanel({ controller }: { controller: EditorController }) {
  const targets = controller.state.project.buildTargets
  const build = controller.state.project.build
  const [selectedTarget, setSelectedTarget] = useState(targets.find((target) => target.active)?.id ?? targets[0]?.id ?? '')
  const [operation, setOperation] = useState<BuildOperation>('cookAndCompile')
  const [runAfterBuild, setRunAfterBuild] = useState(false)
  const [version, setVersion] = useState(build.packageVersion || '0.1.0')
  const [outputRoot, setOutputRoot] = useState(build.packageOutputRoot)

  useEffect(() => {
    if (!targets.some((target) => target.id === selectedTarget)) setSelectedTarget(targets.find((target) => target.active)?.id ?? targets[0]?.id ?? '')
  }, [selectedTarget, targets])
  useEffect(() => { if (build.packageVersion) setVersion(build.packageVersion) }, [build.packageVersion])
  useEffect(() => { if (build.packageOutputRoot) setOutputRoot(build.packageOutputRoot) }, [build.packageOutputRoot])

  const packageReady = operation !== 'packageWindows' || (version.trim().length > 0 && outputRoot.trim().length > 0)
  const startBuild = () => {
    void controller.invoke('build.start', {
      targetId: selectedTarget,
      operation,
      runAfterBuild: operation === 'cookAndCompile' && runAfterBuild,
      ...(operation === 'packageWindows' ? { version: version.trim(), outputRoot: outputRoot.trim() } : {}),
    })
  }

  return <div className="build-panel panel-column">
    <div className="build-content panel-scroll">
      <section><h3>Build Target</h3><div className="build-targets">{targets.map((target) => <button className={selectedTarget === target.id ? 'active' : ''} type="button" key={target.id} onClick={() => setSelectedTarget(target.id)}><Icon name="build" /><span><strong>{target.name}</strong><small>{target.platform} · {target.architecture}</small></span>{target.active && <em>Active</em>}</button>)}{targets.length === 0 && <div className="panel-empty"><Icon name="build" /><span>No build targets are configured for this project</span></div>}</div></section>
      <section className="build-options"><h3>Operation</h3>
        <label className="build-field"><span>Pipeline</span><select value={operation} onChange={(event) => setOperation(event.target.value as BuildOperation)}><option value="validate">Validate Project</option><option value="cookAndCompile">Cook &amp; Compile</option><option value="packageWindows">Package Windows</option></select></label>
        {operation === 'cookAndCompile' && <label className="build-check"><input type="checkbox" checked={runAfterBuild} onChange={(event) => setRunAfterBuild(event.target.checked)} /> Run player after a successful build</label>}
        {operation === 'packageWindows' && <>
          <label className="build-field"><span>Package version</span><input value={version} placeholder="0.1.0" onChange={(event) => setVersion(event.target.value)} /></label>
          <label className="build-field"><span>Output root</span><input value={outputRoot} placeholder="Absolute or project-relative directory" onChange={(event) => setOutputRoot(event.target.value)} /></label>
        </>}
        <p className="build-operation-help">{operation === 'validate' ? 'Checks project, assets, scenes, scripts, and runtime contracts without producing a player.' : operation === 'cookAndCompile' ? 'Cooks project assets and compiles configured scripts for the selected target.' : 'Builds and packages the Windows player, archives, symbols, checksums, and release manifest.'}</p>
      </section>
      {(build.active || build.status || build.output) && <section className="build-progress"><div><strong>{build.status ?? (build.active ? 'Building…' : 'Build finished')}</strong></div>{build.output && <pre>{build.output}</pre>}</section>}
    </div>
    <div className="build-footer">
      {build.cancellable && <button type="button" onClick={() => void controller.invoke('build.cancel', {})}>Cancel</button>}
      <button type="button" disabled={build.active || controller.state.project.runtimeMode !== 'edit'} title={controller.state.project.runtimeMode === 'edit' ? 'Launch the current project player' : 'Stop Play Mode before launching the project player'} onClick={() => void controller.invoke('build.run', {})}>Run Project</button>
      <button className="primary" type="button" disabled={!selectedTarget || build.active || !packageReady} onClick={startBuild}>{OPERATION_LABELS[operation]}</button>
    </div>
  </div>
}
