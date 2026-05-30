# Gate 19 Validation And Acceptance

## Gate Exit Principle

Gate 19 is accepted only when tagged builds can produce tested release artifacts with packaging, profiling, QA automation, diagnostics, and rollback-ready metadata.

## Required Results

- Desktop and mobile packaging paths exist at agreed target scope.
- Asset bundling and version metadata are integrated.
- CPU/GPU/memory profiling can capture baseline data.
- Automated QA runs scene, serialization, asset, hot reload, script, and performance regression checks.
- CI/CD can produce build/test/package artifacts.
- Crash and diagnostics export exists.

## Acceptance Criteria

- [ ] Tagged build creates release artifacts for agreed platforms.
- [ ] Artifacts include version metadata and required assets.
- [ ] QA automation blocks known broken scenes/assets/scripts.
- [ ] Performance baseline report is generated.
- [ ] Crash/log diagnostic bundle can be exported.
- [ ] Release artifacts are signed or checksumed at agreed scope.
- [ ] Rollback procedure is documented and tested for package/update artifacts.

## Automated Checks

- CI build workflow.
- CI test workflow.
- CI package workflow.
- Automated scene runner.
- Asset cook validation.
- Script error recovery tests.
- Performance regression threshold checks.

## Manual Validation

- Install or run packaged build on each target platform in scope.
- Inspect generated symbols, logs, diagnostics, and release metadata.
- Perform a release candidate dry run from tag to archived artifacts.
- Test rollback or previous-build recovery path.

## Blocking Conditions

- Tagged builds cannot be reproduced.
- Release artifacts omit required assets or metadata.
- QA cannot block a known broken scene or asset.
- Profiling data is unavailable for target platforms.
- Crash diagnostics cannot be collected.

## Required Evidence

- CI run link or exported logs.
- Artifact list with checksums/signatures.
- QA report.
- Performance report.
- Diagnostics bundle sample.

## Exit Decision

- Gate owner:
- Date:
- Approved for release candidate: yes/no

