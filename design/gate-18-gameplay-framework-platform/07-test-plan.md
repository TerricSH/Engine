# Gate 18 Test Plan

## Test Strategy

Gate 18 tests prove the high-level gameplay loop, input abstraction, event bus, platform adaptation, and telemetry hooks work together.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G18-F01 Game State Manager | Menu/loading/playing/paused/game-over transitions | Integration | State transitions follow documented rules |
| G18-F02 Input Action Maps | Keyboard/gamepad/touch action queries | Integration | Gameplay reads logical actions |
| G18-F03 Input Rebinding Persistence | Rebind and reload | Integration | Binding persists |
| G18-F04 Gameplay Event Bus | UI/audio/script event delivery | Integration | Consumers receive high-level events |
| G18-F05 Platform Adaptation Layer | Query capabilities/mocks | Unit | Platform differences hidden behind facade |
| G18-F06 Telemetry Hooks | Log local gameplay events | Unit/Integration | Events are recorded without backend service |

## Gate Integration Tests

1. Full gameplay loop playthrough.
2. Input action and UI capture test.
3. Event bus drives UI/audio/script reactions.
4. Platform capability mock test.

## Failure Criteria

- Gameplay uses raw device input directly.
- Event bus is used for engine scheduling.
- Platform SDK calls leak into gameplay scripts.

## Test Fixtures

- `scenes/gate18_game_loop.scene`: menu, loading, gameplay, pause, game-over states.
- `input/gate18_bindings_default.ron`: keyboard/gamepad/touch action mappings.
- `scripts/csharp/Gate18GameplayLoopSample`.
- `platform/gate18_mock_mobile.profile`: mock mobile platform capabilities.

## Executable Integration Cases

### IT-G18-01 Full Gameplay Loop

Steps:
1. Start at menu state.
2. Trigger load level.
3. Enter playing state.
4. Pause and resume.
5. Trigger checkpoint save.
6. Trigger game-over.
7. Return to menu.

Expected:
- State transitions occur in documented order.
- UI/audio/script events are delivered through event bus.

Evidence:
- State transition log.
- Event bus trace.

### IT-G18-02 Input Action Rebinding

Steps:
1. Load default action map.
2. Rebind jump/action key.
3. Save bindings.
4. Reload and query action.

Expected:
- Rebinding persists.
- Gameplay reads logical action, not raw device code.

Evidence:
- Binding file diff.
- Input action log.

### IT-G18-03 Platform Capability Mock

Steps:
1. Run gameplay with mock mobile profile.
2. Query touch, vibration, and performance capabilities.
3. Trigger platform-adapted UI/input behavior.

Expected:
- Gameplay uses capability facade.
- No direct platform SDK calls appear in gameplay scripts.

Evidence:
- Capability query log.
