# Gate 8 Test Plan

## Test Strategy

Gate 8 tests prove secure package install and rollback. Tests must cover rejection paths, interruption recovery, and runtime apply hooks.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G8-F01 Manifest Parser And Verifier | Valid manifest | Unit | Manifest accepted |
| G8-F01 Manifest Parser And Verifier | Invalid signature/version | Unit | Manifest rejected |
| G8-F02 Payload Download Or Local Fetch | Local package staging | Integration | Payloads written only to staging |
| G8-F03 Payload Hash And Signature Verification | Corrupt payload | Unit | Package rejected before install |
| G8-F04 Versioned Package Cache | Cache state after download | Integration | Staged/active/previous packages are distinct |
| G8-F05 Atomic Activation | Simulated crash during activation | Integration | Active package remains old or becomes new; never half-installed |
| G8-F06 Rollback | Failed boot marker | Integration | Previous known-good package restored |
| G8-F07 Runtime Apply Hooks | Apply resource/logic payload | Integration | Registry and logic runtime receive updates |

## Gate Integration Tests

1. Happy-path package install
   - Verify manifest.
   - Download/stage payloads.
   - Activate package.
   - Apply assets/logic.
2. Corrupt package rejection
   - Corrupt payload hash.
   - Confirm no active content changes.
3. Rollback test
   - Mark active package failed.
   - Restore previous known-good.
4. Platform payload test
   - Validate Android-only and iOS-safe payload filtering.

## Required Evidence

- Install logs.
- Rollback logs.
- Corrupt rejection logs.
- Cache directory state snapshots.

## Failure Criteria

- Partial package can become active.
- Corrupt payload reaches asset registry.
- Rollback requires network access.
- iOS path accepts executable payloads.

## Test Fixtures

- `packages/gate08/valid_package/` with manifest, resources, logic payload, hashes.
- `packages/gate08/corrupt_payload/` with one modified payload after hash generation.
- `packages/gate08/incompatible_version/` with invalid engine/script version.
- Temporary package cache root with `staging`, `active`, and `previous` directories.

## Executable Integration Cases

### IT-G8-01 Valid Install And Apply

Steps:
1. Load valid package manifest.
2. Verify manifest and payloads.
3. Stage package into cache.
4. Activate package.
5. Apply assets and logic to runtime stubs.

Expected:
- Active package pointer changes only after verification.
- Asset registry receives only verified payloads.
- Install log records each stage.

Evidence:
- Cache directory snapshot.
- Install log.
- Runtime apply report.

### IT-G8-02 Corrupt Payload Rejection

Steps:
1. Load corrupt package.
2. Run payload verification.
3. Inspect active package pointer.

Expected:
- Package rejected before activation.
- Active package pointer remains unchanged.
- Staging directory is cleaned or marked failed.

Evidence:
- Rejection log.
- Active package pointer before/after.

### IT-G8-03 Rollback Without Network

Steps:
1. Install valid package A.
2. Install valid package B.
3. Mark package B as failed boot.
4. Trigger rollback with network disabled.

Expected:
- Package A becomes active again.
- Rollback reason is logged.

Evidence:
- Rollback log and cache state snapshot.
