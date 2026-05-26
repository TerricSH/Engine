# Gate 7 Test Plan

## Test Strategy

Gate 7 tests are contract and schema focused. They prove mobile runtime profiles, interpreted logic assets, and `MobileHotUpdate-v0` are precise enough for Gate 8 implementation.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G7-F01 Platform Runtime Profiles | Query desktop/android/iOS profiles | Unit | Capabilities and restrictions match documented behavior |
| G7-F01 Platform Runtime Profiles | iOS executable payload policy test | Unit | iOS rejects downloaded executable C# payloads |
| G7-F02 Mobile Script API Subset | API compatibility matrix | Unit | Supported/unsupported APIs are reported clearly |
| G7-F03 Interpreted Logic Asset Contract | Parse example logic asset | Unit | Valid asset parses and validates |
| G7-F03 Interpreted Logic Asset Contract | Invalid transition/parameter test | Unit | Invalid logic asset fails validation |
| G7-F04 MobileHotUpdate-v0 Manifest | Manifest schema validation | Unit | Required fields are enforced |
| G7-F04 MobileHotUpdate-v0 Manifest | Version compatibility tests | Unit | Engine/script/content mismatches reject |
| G7-F05 Android Optional Assembly Policy | Android-only payload filtering | Unit | Payload accepted only under Android policy flag |

## Gate Integration Tests

1. Simulated iOS-safe update contract
   - Load manifest with assets and interpreted logic.
   - Confirm no executable payload is accepted.
2. Android optional payload contract
   - Load same manifest with optional Android assembly payload.
   - Confirm platform filtering works.
3. Cross-platform manifest compatibility
   - Validate manifest against desktop, Android, and iOS profiles.

## Required Evidence

- Manifest examples.
- Profile compatibility test output.
- Logic asset schema validation output.

## Failure Criteria

- iOS profile permits executable C# payloads.
- Manifest cannot express platform-specific payloads.
- Interpreted logic asset schema is too vague to validate.

## Test Fixtures

- `profiles/desktop.profile.ron`, `profiles/android.profile.ron`, `profiles/ios.profile.ron`.
- `logic/state_machine_valid.logic` and `logic/state_machine_invalid.logic`.
- `manifests/mobile_hot_update_valid.json`.
- `manifests/mobile_hot_update_ios_invalid_executable.json`.
- `manifests/mobile_hot_update_version_mismatch.json`.

## Executable Integration Cases

### IT-G7-01 Profile Compatibility Matrix

Steps:
1. Load desktop, Android, and iOS profiles.
2. Validate the same update manifest against all three profiles.
3. Validate Android optional assembly payload only against Android profile.

Expected:
- Desktop and Android accept compatible resource/logic payloads.
- iOS accepts resources/logic and rejects executable C# payload.
- Version mismatches reject on all platforms.

Evidence:
- Compatibility matrix report: `target/test-evidence/gate-07/profile-matrix.json`.

### IT-G7-02 Interpreted Logic Schema

Steps:
1. Parse valid logic asset.
2. Parse invalid logic asset with missing state/transition target.
3. Run deterministic validation.

Expected:
- Valid logic asset passes.
- Invalid asset reports schema location and reason.

Evidence:
- Logic validation report.

### IT-G7-03 Manifest Schema Contract

Steps:
1. Parse valid manifest.
2. Remove required fields one at a time.
3. Validate hash/signature placeholder fields and compatibility fields.

Expected:
- Required fields are enforced.
- Missing compatibility information rejects package before Gate 8 installer.

Evidence:
- Manifest validation log.
