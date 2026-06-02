# Gate 19 Test Plan

## Test Strategy

Gate 19 tests prove release artifacts are reproducible, tested, profiled, packaged, and diagnosable.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G19-F01 Reproducible Build Scripts | Build same tag twice | CI/Manual | Equivalent artifacts and metadata |
| G19-F02 Platform Packaging | Package each target platform | CI/Manual | Build launches on target |
| G19-F03 Asset Packaging | Package cooked assets only | CI | Release does not require source assets |
| G19-F04 Profiling Baselines | Capture CPU/GPU/memory metrics | Runtime | Report generated |
| G19-F05 QA Automation | Run scene/asset/script/perf tests | CI | Broken content blocks build |
| G19-F06 CI/CD Release Artifacts | Tagged release workflow | CI | Artifacts archived with logs |
| G19-F07 Crash And Diagnostics Export | Generate sample diagnostic bundle | Integration | Symbols resolve release ID |

## Gate Integration Tests

1. Release candidate dry run from tag.
2. Install/run packaged build per platform in scope.
3. QA suite blocks known broken scene/asset/script.
4. Profiling report generated and archived.
5. Crash diagnostic bundle resolves with archived symbols.

## Failure Criteria

- Release requires manual untracked steps.
- Generated assets are hand-edited.
- Symbols or release metadata are missing.
- CI skips platform-specific smoke tests.

## Test Fixtures

> **Fixture implementation status: Pending.** The fixtures below are referenced by integration tests and manual validation. As of the current gate review, the following items do not yet exist on disk and must be created during the build/QA infrastructure implementation phase.

- `release/gate19_test_project/`: minimal game project with scene, assets, script, UI, audio. *(not yet created)*
- Tagged test version such as `v0.0.0-gate19-test`. *(not yet created)*
- Known broken scene, asset, and script fixtures for negative QA. *(not yet created)*
- Mock crash dump or deliberate crash mode for diagnostics. *(not yet created)*

## Executable Integration Cases

> **Implementation status: Pending.** The integration tests IT-G19-01/02/03 are not yet implemented. They are described below as the target specification; the CI infrastructure, test project, and fixtures must be created before gate exit.

### IT-G19-01 Release Candidate Dry Run

Steps:
1. Create or checkout test tag.
2. Run CI-equivalent build locally or in CI.
3. Cook assets.
4. Package target platforms in scope.
5. Archive artifacts, symbols, checksums, and metadata.

Expected:
- Artifacts are generated without manual untracked steps.
- Artifact layout matches release contract.

Evidence:
- CI/build log.
- Artifact manifest.

### IT-G19-02 QA Blocks Broken Content

Steps:
1. Run QA suite with valid project.
2. Run QA suite with known broken scene, asset, and script fixtures.

Expected:
- Valid project passes.
- Broken fixtures fail with actionable diagnostics.

Evidence:
- QA reports for pass and fail runs.

### IT-G19-03 Profiling And Crash Diagnostics

Steps:
1. Run profiling capture for validation project.
2. Generate sample crash/diagnostic bundle.
3. Resolve crash with archived symbols and release ID.

Expected:
- Performance report is generated.
- Diagnostic bundle maps to release metadata and symbols.

Evidence:
- Performance report.
- Diagnostic bundle sample.
- Symbol resolution log.
