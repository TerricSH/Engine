# Gate 16 Session Prompts

Gate 16 implements runtime audio. Keep audio independent from renderer and UI internals.

## Session 16A: Audio Asset And Backend Owner

Goal: Implement audio asset cook/load and backend abstraction.

Owns:
- `crates/engine-audio` asset/backend modules

Must not edit:
- UI or renderer internals

Expected output:
- cooked audio assets load
- backend initializes/shuts down

Validation:
- asset cook/load tests

## Session 16B: Playback Mixer Owner

Goal: Implement playback instances and mixer groups.

Owns:
- playback and mixer modules

Must not edit:
- audio asset schema without coordination

Expected output:
- play/pause/stop/volume/group routing

Validation:
- playback tests

## Session 16C: Spatial Editor C# Owner

Goal: Implement listener/source components, spatial attenuation, editor preview, and C# APIs.

Owns:
- spatial modules
- editor plugin
- C# bindings

Must not edit:
- backend device code

Expected output:
- 3D audio attenuation and C# playback sample

Validation:
- spatial scene test
