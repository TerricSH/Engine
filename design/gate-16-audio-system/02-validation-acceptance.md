# Gate 16 Validation And Acceptance

## Gate Exit Principle

Gate 16 is accepted only when audio assets can be cooked, loaded, played, spatialized, serialized, and controlled from editor/C# without coupling to renderer or UI internals.

## Required Results

- Audio asset cookers support agreed source formats.
- Audio runtime supports load, play, stop, pause, volume, and basic mixer routing.
- Listener and audio source components exist.
- Spatial attenuation works with ECS transforms.
- Editor fields and C# playback APIs exist.

## Acceptance Criteria

- [ ] 2D sound plays in validation scene.
- [ ] 3D sound attenuates based on listener/source distance.
- [ ] Audio source components save/load.
- [ ] C# can call `PlaySound`, `StopSound`, and `SetVolume` or agreed equivalents.
- [ ] Editor can preview an audio source.
- [ ] Audio asset validation reports missing or unsupported files.
- [ ] Audio runtime does not depend on UI or renderer internals.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- `cargo test --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- Audio asset cook/load tests.
- Mixer state tests.
- Spatial attenuation tests.
- Scene serialization tests for listener/source components.

## Manual Validation

- Play 2D and 3D audio in a validation scene.
- Move listener/source and verify attenuation changes.
- Preview audio from editor.
- Trigger audio playback from C#.

## Blocking Conditions

- Audio playback requires renderer or UI internals.
- Audio components cannot save/load.
- Missing audio assets fail silently.
- Audio backend cannot be abstracted behind runtime API.

## Required Evidence

- Audio validation scene notes.
- C# playback log.
- Audio asset cook output.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 17: yes/no

