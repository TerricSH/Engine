# Gate 8: Hot Update Package Implementation

## Purpose

Implement signed hot update packages after mobile and update contracts are frozen. This gate handles download, verification, install, rollback, and platform-specific payloads.

## Entry Sync Point

- `MobileHotUpdate-v0` is frozen.

## Parallel Workstreams

1. Package Download And Verification
   - Owns manifest fetch, file download, hash/signature checks, and compatibility rejection.
2. Install And Rollback
   - Owns local cache, versioned install, previous-known-good rollback, and partial download recovery.
3. Resource And Logic Updates
   - Owns asset registry update integration and interpreted logic asset reload.
4. Android Optional Assembly Payload
   - Owns Android-only C# assembly payload path if approved.
   - Must not affect iOS assumptions.

## Contracts To Preserve

- Hot update manifest schema
- Engine/script API version compatibility rules
- Signed package verification rules

## Exit Condition

- Signed packages can update resources and interpreted logic assets.
- Corrupt or incompatible packages are rejected.
- Rollback works.
- Android assembly update remains optional and platform-specific.

## Parallel Safety Notes

- Package installation must be atomic.
- Rollback must preserve the last known good package.
- iOS update path remains resource/logic-asset only.
