import type {
  AnimationSnapshot,
  BuildSnapshot,
  EditorTelemetry,
  EditorTelemetryEvent,
  FrameStatsSnapshot,
  PerformanceSnapshot,
  ProjectSnapshot,
  TerrainSnapshot,
} from './protocol'

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}

function hasOwn(value: Record<string, unknown>, keys: readonly string[]): boolean {
  return keys.every((key) => Object.hasOwn(value, key))
}

function isFiniteNonNegative(value: unknown): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
}

function isCounter(value: unknown): value is number {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0
}

function isOptionalString(value: unknown): value is string | undefined {
  return value === undefined || typeof value === 'string'
}

function isFrameStats(value: unknown): value is FrameStatsSnapshot {
  if (!isRecord(value) || !hasOwn(value, [
    'frameTimeMs', 'drawCalls', 'triangles', 'physicsBodies', 'animationCount', 'navAgents', 'assetCount',
  ])) return false
  return isFiniteNonNegative(value.frameTimeMs)
    && isCounter(value.drawCalls)
    && isCounter(value.triangles)
    && isCounter(value.physicsBodies)
    && isCounter(value.animationCount)
    && isCounter(value.navAgents)
    && isCounter(value.assetCount)
}

function isPerformance(value: unknown): value is PerformanceSnapshot {
  if (!isRecord(value) || !hasOwn(value, ['current', 'history']) || !Array.isArray(value.history)) return false
  return isFrameStats(value.current) && value.history.every(isFrameStats)
}

function isAnimation(value: unknown): value is AnimationSnapshot {
  if (!isRecord(value) || !hasOwn(value, [
    'availableSkeletons', 'availableClips', 'playbackTime', 'duration', 'playing', 'looping', 'speed', 'events',
  ])) return false
  return Array.isArray(value.availableSkeletons)
    && value.availableSkeletons.every((entry) => typeof entry === 'string')
    && Array.isArray(value.availableClips)
    && value.availableClips.every((entry) => typeof entry === 'string')
    && isOptionalString(value.selectedSkeleton)
    && isOptionalString(value.selectedClip)
    && isFiniteNonNegative(value.playbackTime)
    && isFiniteNonNegative(value.duration)
    && isFiniteNonNegative(value.speed)
    && typeof value.playing === 'boolean'
    && typeof value.looping === 'boolean'
    && Array.isArray(value.events)
    && value.events.every((event) => isRecord(event)
      && hasOwn(event, ['time', 'name'])
      && isFiniteNonNegative(event.time)
      && typeof event.name === 'string')
}

function isBuild(value: unknown): value is BuildSnapshot {
  if (!isRecord(value) || !hasOwn(value, [
    'active', 'cancellable', 'output', 'packageVersion', 'packageOutputRoot',
  ])) return false
  return typeof value.active === 'boolean'
    && typeof value.cancellable === 'boolean'
    && isOptionalString(value.status)
    && typeof value.output === 'string'
    && typeof value.packageVersion === 'string'
    && typeof value.packageOutputRoot === 'string'
}

function isTerrain(value: unknown): value is TerrainSnapshot {
  if (!isRecord(value) || !hasOwn(value, [
    'available', 'enabled', 'seed', 'chunkSize', 'baseResolution', 'heightScale',
    'frequency', 'octaves', 'lacunarity', 'gain', 'domainWarpAmplitude',
    'domainWarpFrequency', 'skirtDepth', 'collisionEnabled', 'lodDistances', 'lodHysteresis', 'runtime',
  ])) return false
  if (typeof value.available !== 'boolean' || typeof value.enabled !== 'boolean'
    || typeof value.seed !== 'string' || typeof value.collisionEnabled !== 'boolean'
    || !Array.isArray(value.lodDistances) || !value.lodDistances.every(isFiniteNonNegative)
    || !isFiniteNonNegative(value.lodHysteresis)
    || !isRecord(value.runtime)) return false
  const finiteFields = [
    'chunkSize', 'baseResolution', 'heightScale', 'frequency', 'octaves',
    'lacunarity', 'gain', 'domainWarpAmplitude', 'domainWarpFrequency', 'skirtDepth',
  ] as const
  if (!finiteFields.every((field) => {
    const fieldValue = value[field]
    return typeof fieldValue === 'number' && Number.isFinite(fieldValue)
  })) return false
  if (!Number.isSafeInteger(value.baseResolution) || !Number.isSafeInteger(value.octaves)) return false
  if (!/^(0|[1-9][0-9]*)$/.test(value.seed)) return false
  return hasOwn(value.runtime, [
    'queued', 'generating', 'readyToCommit', 'resident', 'failed', 'residentBytes',
    'staleResultsDiscarded', 'cancelled', 'generated', 'committed', 'evicted',
    'lastTickCommittedBytes', 'lastGenerationMicros',
  ]) && Object.values(value.runtime).every(isFiniteNonNegative)
}

export function isCompleteEditorTelemetry(value: unknown): value is EditorTelemetry {
  if (!isRecord(value) || !hasOwn(value, ['performance', 'animation', 'build', 'terrain'])) return false
  return isPerformance(value.performance)
    && isAnimation(value.animation)
    && isBuild(value.build)
    && isTerrain(value.terrain)
}

export function telemetryMatchesAuthoritativeSnapshot(
  authoritativeSnapshotRevision: number | undefined,
  telemetryRevision: number,
): boolean {
  return authoritativeSnapshotRevision !== undefined
    && isCounter(telemetryRevision)
    && telemetryRevision === authoritativeSnapshotRevision
}

/** Replace complete high-frequency domains while preserving authoritative project state. */
export function mergeProjectTelemetry(
  project: ProjectSnapshot,
  event: EditorTelemetryEvent,
): ProjectSnapshot {
  if (event.sessionId !== project.sessionId
    || event.revision !== project.revision
    || !isCounter(event.sequence)
    || !isCounter(event.revision)
    || !isCompleteEditorTelemetry(event.params)) {
    return project
  }
  return {
    ...project,
    performance: event.params.performance,
    animation: event.params.animation,
    build: event.params.build,
    terrain: event.params.terrain,
  }
}
