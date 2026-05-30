# Gate 6 Feature Requirements And Execution Boundaries

## Gate Objective

Add fast development iteration through hot reload, incremental recook, safe runtime resource replacement, and editor/sandbox diagnostics.

## Required Features

### G6-F01 File Watch Or Manual Reload Trigger

Required behavior:
- Detect source/cooked changes in development mode or via explicit reload command.
- Debounce/coalesce noisy file events.

Minimum output:
- A changed asset path becomes a reload request.

### G6-F02 Incremental Recook

Required behavior:
- Use dependency graph to determine changed assets and reverse dependencies.
- Recook only affected assets.
- Validate recooked output before runtime replacement.

Minimum output:
- Broken recook reports error and keeps old asset active.

### G6-F03 Reload Queue

Required behavior:
- Track reload states: detected, recooking, cooked, queued, applying, applied, failed.
- Apply reloads only at safe frame boundaries.

Minimum output:
- Editor and sandbox can observe reload state.

### G6-F04 Runtime Resource Reload

Required behavior:
- Reload textures, materials, shaders, and scenes.
- Recreate Vulkan resources/pipelines safely.
- Delay destruction of old GPU resources until in-flight frames are safe.

Minimum output:
- Visible scene updates without process restart.

### G6-F05 Diagnostics

Required behavior:
- Show changed path, affected asset ID, dependency chain, reload state, error messages, and latency.

Minimum output:
- A failed shader or asset reload is understandable without reading engine logs only.

## Target Effects

- Developers can iterate on common content without restarting.
- Runtime remains stable under reload success and failure.
- Editor and sandbox share reload diagnostics.

## Explicit Non-Goals

- No production hot update package system.
- No arbitrary live C# code replacement.
- No background streaming architecture.
- No complex scene merge system.

## AI Execution Rules

- Do not change `AssetRegistry-v0` semantics.
- Reload at frame boundaries only.
- Preserve last valid resources on failure.
- Do not destroy in-flight GPU resources.
- Auto-reload must be disableable.

## Completion Signal

Gate 6 is complete when texture/material/shader/scene reloads work, failures preserve previous state, diagnostics are visible, and validation layers remain clean.
