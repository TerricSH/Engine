# Gate 18 Feature Requirements And Execution Boundaries

## Gate Objective

Implement high-level gameplay framework and platform adaptation so a complete game loop can run across desktop and mobile-facing input/platform abstractions.

## Required Features

### G18-F01 Game State Manager

Required behavior:
- Implement menu, loading, playing, paused, game-over states and transition rules.
- Expose C# hooks for state changes.

Minimum output:
- Validation project moves through full gameplay loop.

### G18-F02 Input Action Maps

Required behavior:
- Implement logical input actions with value types, bindings, mapping contexts, and query APIs.
- Support keyboard/mouse and at least one gamepad or touch path at agreed scope.

Minimum output:
- Gameplay reads actions, not raw devices.

### G18-F03 Input Rebinding Persistence

Required behavior:
- Save/load binding overrides at agreed scope.

Minimum output:
- Rebound action persists across restart or scene reload.

### G18-F04 Gameplay Event Bus

Required behavior:
- Implement gameplay-level event dispatcher for UI, audio, score, progression, quest/combat, and game state events.

Minimum output:
- UI and audio react to gameplay events without direct subsystem coupling.

### G18-F05 Platform Adaptation Layer

Required behavior:
- Expose platform capabilities such as touch, gamepad availability, vibration, performance mode, and mobile device flags.

Minimum output:
- Gameplay can query capabilities through a mockable API.

### G18-F06 Telemetry Hooks

Required behavior:
- Implement local event logging and stub submission interface.

Minimum output:
- Telemetry events appear in local logs without external service dependency.

## Target Effects

- Menu/load/play/pause/save/game-over loop works.
- Gameplay input is device-independent.
- UI/audio/gameplay communicate through high-level events.
- Platform-specific behavior is behind capability APIs.

## Explicit Non-Goals

- No multiplayer.
- No achievements or leaderboards.
- No cloud saves.
- No server analytics backend.
- No platform store SDK integration.

## AI Execution Rules

- Event bus is for gameplay events, not engine scheduling.
- Gameplay scripts must not call platform SDKs directly.
- Input actions must be serializable and testable.
- Platform capabilities must be mockable.

## Completion Signal

Gate 18 is complete when the full gameplay loop runs, input action maps work across target devices, UI/audio/gameplay communicate through events, and platform adaptation is demonstrated.
