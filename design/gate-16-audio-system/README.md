# Gate 16: Audio System Foundation

## Purpose

Add audio asset loading, playback, simple mixing, and spatial audio. This gate should stay independent from renderer, UI, and editor internals.

## Entry Sync Point

- Asset registry is stable.
- ECS extension surfaces are stable.
- Scene serialization is stable.
- Can run in parallel with Gate 15 if UI and audio own separate crates.

## Parallel Workstreams

1. Audio Asset Pipeline
   - Owns `crates/engine-audio` and audio asset cookers.
   - Supports WAV/OGG source import and cooked audio metadata.
2. Playback And Mixer Core
   - Adds sound asset load/play/stop/pause, volume, channel/group routing, and simple mixer abstraction.
3. Spatial Audio
   - Adds listener and audio source components with distance attenuation, position tracking, and simple falloff.
4. Editor And C# Support
   - Editor fields for audio source, looping, volume, spatial settings, and preview.
   - C# APIs for `PlaySound`, `StopSound`, `SetVolume`, and audio asset references.

## Exit Condition

- 2D and simple 3D audio play in a scene.
- Listener/source transforms affect spatial attenuation.
- Audio components save/load and expose editor/C# controls.
- Audio remains isolated from renderer and UI internals.

## Non-Goals

- Voice chat, advanced DSP graph, music authoring timeline, live mixer editor, and networked audio sync.

## Parallel Safety Notes

- Audio asset cooking uses asset extension APIs.
- Audio source/listener components register through subsystem extension APIs.
- Audio must not depend on UI runtime or editor UI internals.
