# Gate 10 Session Prompts

Gate 10 can run physics and animation in parallel after Gate 9. Keep crates isolated.

## Session 10A: Physics Foundation Owner

Goal: Implement engine-owned physics components, world, backend adapter, queries, and events.

Owns:
- `crates/engine-physics`
- first physics backend crate

Must not edit:
- animation crates
- renderer backend internals
- core ECS schema outside extension registration

Expected output:
- rigid bodies/colliders/materials
- fixed timestep physics world
- raycast/overlap/sweep
- collision/trigger events

Validation:
- physics validation scene
- query/event tests

## Session 10B: Animation Foundation Owner

Goal: Implement skeleton/clip assets, animation player, evaluator, bone palette extraction.

Owns:
- `crates/engine-animation`
- animation cooker modules through asset extension

Must not edit:
- physics crates
- renderer backend internals

Expected output:
- single-clip skeletal playback
- `RendererInput-v0.skinned_items` output (per `FD-007`)

Validation:
- animation validation scene
- skeleton/clip compatibility tests

## Session 10C: Editor/C# Debug Integration Owner

Goal: Add minimal editor fields, C# APIs, and debug draw for physics/animation.

Owns:
- subsystem editor plugins
- subsystem C# bindings
- debug draw providers

Must not edit:
- physics/animation core internals except through public APIs

Expected output:
- collider/skeleton debug draw
- collision callback and animation playback C# samples

Validation:
- editor visibility checks
- C# sample logs
