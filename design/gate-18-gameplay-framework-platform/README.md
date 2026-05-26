# Gate 18: Gameplay Framework And Platform Adaptation

## Purpose

Add the high-level gameplay framework that ties UI, audio, input, scripting, and platform differences together. This gate establishes a complete gameplay loop without introducing multiplayer or live service features.

## Entry Sync Point

- UI is stable.
- Audio is stable.
- Character controller is stable.
- Navigation/AI is stable.
- Prefab and scripting foundations are stable.

## Parallel Workstreams

1. Game State Manager
   - Adds menu/loading/playing/paused/game-over states, transitions, save checkpoints, and C# hooks.
2. Input Action Maps
   - Adds platform-agnostic actions, keyboard/gamepad/touch bindings, rebinding, and C# query APIs.
3. Gameplay Event Bus
   - Adds gameplay-level event dispatcher for score, UI, audio, quest, combat, and state changes.
   - Not for engine-internal low-level events.
4. Platform Adaptation Layer
   - Adds mobile touch controls, capability checks, vibration hooks, performance mode toggles, and platform-specific facades.
5. Analytics And Lightweight Telemetry Hooks
   - Adds local event logging and stub submission interfaces.
   - Keeps backend analytics integration deferred.

## Exit Condition

- Complete gameplay loop works: menu, load scene, play, pause, save/checkpoint, game-over, return to menu.
- Input actions work across desktop and mobile targets.
- UI/audio/gameplay communicate through the event bus without tight coupling.

## Non-Goals

- Multiplayer, achievements, leaderboards, server analytics, cloud saves, and platform store SDK integration.

## Parallel Safety Notes

- Gameplay framework uses public APIs from UI, audio, character, AI, prefab, and scripting systems.
- Event bus is for gameplay-level events, not engine-internal scheduling.
- Platform adaptation should hide platform differences behind capability APIs.
