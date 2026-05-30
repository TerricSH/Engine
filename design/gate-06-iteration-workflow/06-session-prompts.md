# Gate 6 Session Prompts

Gate 6 adds iteration speed. Sessions must not mutate frozen `AssetRegistry-v0` or `ScriptAPI-v0` semantics.

## Session 6A: Hot Reload Pipeline Owner

Goal: Implement watch, incremental recook, reload queue, and diagnostics.

Owns:
- reload modules in `crates/engine-asset`
- reload diagnostics docs/tests

Must not edit:
- asset ID/cooked manifest semantics
- editor panel internals except diagnostics API

Expected output:
- file watcher/manual reload trigger
- dependency-based recook
- reload state machine
- failed reload fallback

Validation:
- watcher/recook/reload tests
- broken asset fallback test

## Session 6B: Vulkan Resource Reload Owner

Goal: Apply safe runtime reload for GPU resources.

Owns:
- reload modules in `crates/render-vulkan`

Must not edit:
- asset registry semantics
- OpenGL/DX12 internals except stubs if coordinated

Expected output:
- texture/material/shader reload at frame boundaries
- delayed destruction for in-flight resources
- pipeline recreation on shader success, fallback on failure

Validation:
- repeated reload under validation layers
- shader failure keeps old pipeline

## Session 6C: Editor And Sandbox Diagnostics Owner

Goal: Surface reload and script diagnostics to users.

Owns:
- editor diagnostics panels
- sandbox diagnostics output

Must not edit:
- reload state machine semantics

Expected output:
- changed path, affected asset, state, errors, latency visible

Validation:
- manual reload workflow shows useful diagnostics
