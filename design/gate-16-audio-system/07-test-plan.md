# Gate 16 Test Plan

## Test Strategy

Gate 16 tests prove audio assets, playback, mixing, spatial audio, editor preview, and C# controls work without renderer/UI coupling.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G16-F01 Audio Asset Pipeline | Cook/load WAV or OGG | Integration | Audio asset loads through registry |
| G16-F02 Audio Backend Abstraction | Init/shutdown backend | Runtime | Backend starts and stops cleanly |
| G16-F03 Playback Instances | Play/pause/stop sound | Runtime | Playback state changes correctly |
| G16-F04 Mixer Groups | Adjust group volume | Runtime | Category volume changes without affecting others |
| G16-F05 Spatial Audio | Move listener/source | Runtime | Attenuation changes with distance |
| G16-F06 Editor And C# Controls | Preview and C# playback | Manual/Integration | Editor preview and C# play call work |

## Gate Integration Tests

1. 2D audio playback scene.
2. 3D spatial audio scene.
3. Mixer group routing test.
4. C# audio trigger scene.

## Failure Criteria

- Audio callback blocks on asset loading.
- C# receives backend handles.
- Audio depends on renderer or UI internals.

## Test Fixtures

- `assets/audio/gate16_beep.wav` or equivalent short 2D sound.
- `assets/audio/gate16_loop.ogg` or equivalent looping sound.
- `scenes/gate16_spatial.scene`: listener and moving source.
- `scripts/csharp/Gate16AudioSample`.

## Executable Integration Cases

### IT-G16-01 Playback State

Steps:
1. Load short audio asset.
2. Play, pause, resume, and stop.
3. Query playback state after each operation.

Expected:
- State transitions match requested operations.
- Playback handle remains engine-owned.

Evidence:
- Playback state log.

### IT-G16-02 Spatial Attenuation

Steps:
1. Load spatial scene.
2. Move source from near to far distances.
3. Record computed gain/attenuation values.

Expected:
- Gain decreases according to selected attenuation curve.
- Listener/source transforms drive spatial parameters.

Evidence:
- Spatial parameter trace.

### IT-G16-03 C# And Editor Preview

Steps:
1. Preview sound from editor.
2. Trigger sound from C# sample.

Expected:
- Both paths use public audio API.
- No backend handle crosses into C#.

Evidence:
- Editor preview log.
- C# playback log.
