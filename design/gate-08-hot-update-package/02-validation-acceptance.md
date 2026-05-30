# Gate 8 Validation And Acceptance

## Gate Exit Principle

Gate 8 is accepted only when signed update packages can be downloaded, verified, installed, rejected, and rolled back without corrupting the runtime asset or logic state.

## Required Results

- Package downloader or local package fetch path exists.
- Manifest verification checks engine version, script API version, hashes, signatures, and platform payload compatibility.
- Versioned install cache exists.
- Rollback to previous known-good package works.
- Resource and interpreted logic updates integrate with the asset registry/update path.
- Android optional assembly payload remains platform-specific.

## Acceptance Criteria

- [ ] Valid package installs successfully.
- [ ] Corrupt package is rejected before install.
- [ ] Incompatible engine version is rejected.
- [ ] Incompatible script API version is rejected.
- [ ] Partial download can be retried or cleaned safely.
- [ ] Rollback restores previous known-good assets and logic metadata.
- [ ] iOS package path excludes executable C# payloads.

## Automated Checks

- Manifest parse and validation tests.
- Hash/signature rejection tests.
- Compatibility matrix tests.
- Install/rollback tests using temporary package cache.
- Corrupt and partial package recovery tests.

## Manual Validation

- Install a sample package and launch the sandbox using updated resources.
- Install a broken package and confirm the old package remains active.
- Verify Android-only payload is ignored or rejected on non-Android targets.

## Blocking Conditions

- Package install is not atomic.
- Rollback cannot recover the previous package.
- Corrupt payload can reach runtime systems.
- Platform-specific payloads are not isolated.

## Required Evidence

- Package schema example.
- Install and rollback command logs.
- Corrupt package rejection output.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 9: yes/no

