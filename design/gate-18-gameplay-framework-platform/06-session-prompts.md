# Gate 18 Session Prompts

Gate 18 coordinates gameplay systems through public APIs and event/action abstractions.

## Session 18A: Game State And Input Owner

Goal: Implement game state manager and input action maps.

Owns:
- game state modules
- input action modules

Must not edit:
- platform backend internals except through input events

Expected output:
- menu/loading/playing/paused/game-over flow
- logical action queries and rebinding

Validation:
- gameplay loop and input tests

## Session 18B: Event Bus And Platform Adaptation Owner

Goal: Implement gameplay event bus and platform capability facade.

Owns:
- gameplay event bus
- platform capability facade

Must not edit:
- subsystem internals directly

Expected output:
- UI/audio/gameplay communicate through events
- mobile/desktop capabilities queryable

Validation:
- event delivery and capability tests

## Session 18C: Telemetry And Integration Owner

Goal: Implement local telemetry hooks and integration scene.

Owns:
- telemetry stub modules
- validation gameplay scene

Must not edit:
- analytics backend integrations

Expected output:
- local telemetry logs and full gameplay loop demo

Validation:
- integration playthrough
