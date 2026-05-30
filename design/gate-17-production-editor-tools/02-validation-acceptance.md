# Gate 17 Validation And Acceptance

## Gate Exit Principle

Gate 17 is accepted only when the editor is practical for production authoring and subsystem tools are delivered through plugin surfaces rather than hardcoded into core editor modules.

## Required Results

- Advanced transform/debug gizmos exist.
- Asset browser and assignment workflow exist.
- Material/shader editing tools exist at agreed scope.
- Animation preview tools exist.
- Prefab composition tools exist.
- Performance inspector panel exists.

## Acceptance Criteria

- [ ] Move/rotate/scale gizmos edit selected entities correctly.
- [ ] Snapping/pivot behavior works at agreed minimum scope.
- [ ] Asset browser can search and assign assets.
- [ ] Material changes preview in editor.
- [ ] Animation preview can scrub/play clips.
- [ ] Prefab override diff/apply/revert UI works.
- [ ] Performance inspector shows frame stats, draw calls, memory summary, asset count, physics body count, animation count, and AI agent count.
- [ ] Editor tools consume subsystem plugin surfaces.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- `cargo test --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- Editor command history tests.
- Asset assignment tests.
- Prefab editor workflow tests.

## Manual Validation

- Build a small scene using production editor tools only.
- Assign assets through browser and verify scene save/load.
- Edit material and preview result.
- Preview animation and inspect debug gizmos.

## Blocking Conditions

- Core editor hardcodes physics/animation/audio/UI internals instead of plugin surfaces.
- Editor edits break scene or prefab serialization.
- Asset assignment bypasses asset registry.
- Performance panel cannot run in normal editor sessions.

## Required Evidence

- Editor workflow capture.
- Example authored scene.
- Plugin integration review note.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 18: yes/no

