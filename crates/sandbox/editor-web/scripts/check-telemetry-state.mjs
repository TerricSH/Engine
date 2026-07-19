import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'

const source = await readFile(new URL('../src/bridge/telemetry.ts', import.meta.url), 'utf8')
const compiled = ts.transpileModule(source, {
  compilerOptions: { module: ts.ModuleKind.ES2022, target: ts.ScriptTarget.ES2022 },
}).outputText
const moduleUrl = `data:text/javascript;base64,${Buffer.from(compiled).toString('base64')}`
const { isCompleteEditorTelemetry, mergeProjectTelemetry, telemetryMatchesAuthoritativeSnapshot } = await import(moduleUrl)

const oldPerformance = { current: { frameTimeMs: 20, drawCalls: 1, triangles: 12, physicsBodies: 0, animationCount: 0, navAgents: 0, assetCount: 2 }, history: [] }
const oldAnimation = { availableSkeletons: [], availableClips: [], playbackTime: 0, duration: 0, playing: false, looping: false, speed: 1, events: [] }
const oldBuild = { active: false, cancellable: false, output: '', packageVersion: '0.1.0', packageOutputRoot: 'dist' }
const oldTerrain = { available: false, enabled: true, seed: '0', chunkSize: 64, baseResolution: 65, heightScale: 24, frequency: 0.008, octaves: 5, lacunarity: 2, gain: 0.5, domainWarpAmplitude: 0, domainWarpFrequency: 0.01, skirtDepth: 4, collisionEnabled: true, lodDistances: [160, 320, 640], lodHysteresis: 16, runtime: { queued: 0, generating: 0, readyToCommit: 0, resident: 0, failed: 0, residentBytes: 0, staleResultsDiscarded: 0, cancelled: 0, generated: 0, committed: 0, evicted: 0, lastTickCommittedBytes: 0, lastGenerationMicros: 0 } }
const project = {
  sessionId: 'session-a', revision: 7, hierarchy: [{ id: 'keep-authoritative-state' }],
  performance: oldPerformance, animation: oldAnimation, build: oldBuild, terrain: oldTerrain,
}
const telemetry = {
  protocol: 'EngineEditorIpc-v2', sessionId: 'session-a', revision: 7,
  // The gap from an earlier event is safe because each high-frequency domain is complete.
  sequence: 42, event: 'editor.telemetry',
  params: {
    performance: { current: { frameTimeMs: 8.5, drawCalls: 4, triangles: 24, physicsBodies: 2, animationCount: 1, navAgents: 3, assetCount: 9 }, history: [oldPerformance.current] },
    animation: { availableSkeletons: ['hero'], availableClips: ['idle'], selectedSkeleton: 'hero', selectedClip: 'idle', playbackTime: 0.5, duration: 2, playing: true, looping: true, speed: 1, events: [{ time: 1, name: 'step' }] },
    build: { active: true, cancellable: true, status: 'Compiling', output: 'crate 3/4', packageVersion: '0.2.0', packageOutputRoot: 'releases' },
    terrain: { ...oldTerrain, available: true, entityId: 'terrain', runtime: { ...oldTerrain.runtime, resident: 4, residentBytes: 8192 } },
  },
}

assert.equal(isCompleteEditorTelemetry(telemetry.params), true)
const merged = mergeProjectTelemetry(project, telemetry)
assert.equal(merged.hierarchy, project.hierarchy, 'telemetry must preserve authoritative project domains')
assert.equal(merged.performance, telemetry.params.performance)
assert.equal(merged.animation, telemetry.params.animation)
assert.equal(merged.build, telemetry.params.build)
assert.equal(merged.terrain, telemetry.params.terrain)

assert.equal(isCompleteEditorTelemetry({ ...telemetry.params, performance: { current: telemetry.params.performance.current } }), false)
assert.equal(isCompleteEditorTelemetry({ ...telemetry.params, animation: { ...telemetry.params.animation, playbackTime: Number.NaN } }), false)
assert.equal(mergeProjectTelemetry(project, { ...telemetry, revision: 8 }), project)
assert.equal(mergeProjectTelemetry(project, { ...telemetry, sessionId: 'session-b' }), project)

// A mutation response advances the command revision first. Telemetry at that revision must wait
// until the complete project.changed snapshot becomes authoritative, then becomes deliverable.
let authoritativeSnapshotRevision = 7
const commandResponseRevision = 8
assert.equal(telemetryMatchesAuthoritativeSnapshot(authoritativeSnapshotRevision, commandResponseRevision), false)
authoritativeSnapshotRevision = 8
assert.equal(telemetryMatchesAuthoritativeSnapshot(authoritativeSnapshotRevision, commandResponseRevision), true)
assert.notEqual(
  mergeProjectTelemetry({ ...project, revision: 8 }, { ...telemetry, revision: 8 }),
  project,
)

console.log('Editor telemetry: complete domains safely replace state across sequence gaps.')
