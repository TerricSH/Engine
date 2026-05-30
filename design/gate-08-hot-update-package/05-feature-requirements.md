# Gate 8 Feature Requirements And Execution Boundaries

## Gate Objective

Implement secure hot update package handling on top of `MobileHotUpdate-v0`: fetch, verify, stage, activate, apply, and roll back.

## Required Features

### G8-F01 Manifest Parser And Verifier

Required behavior:
- Parse `MobileHotUpdate-v0` manifests.
- Verify manifest signature, engine/script/content compatibility, package version, platform payload rules, and required fields.

Minimum output:
- Valid manifest accepts; invalid, incompatible, or unsigned manifest rejects with diagnostics.

### G8-F02 Payload Download Or Local Fetch

Required behavior:
- Fetch package payloads into a staging location.
- Support local test packages for deterministic testing.
- Track download status and partial results.

Minimum output:
- Payloads never write directly into active package directories.

### G8-F03 Payload Hash And Signature Verification

Required behavior:
- Verify payload hashes and signatures before activation.
- Reject missing, corrupt, or wrong-platform payloads.

Minimum output:
- Corrupt payload test rejects package before install.

### G8-F04 Versioned Package Cache

Required behavior:
- Store staged, active, and previous known-good packages separately.
- Persist enough state to recover after interruption.

Minimum output:
- Active package pointer can be inspected and restored.

### G8-F05 Atomic Activation

Required behavior:
- Activate a verified package via metadata switch or equivalent atomic operation.
- Do not destructively overwrite active content.

Minimum output:
- Interrupted activation leaves either old or new package valid, not half-installed state.

### G8-F06 Rollback

Required behavior:
- Restore previous known-good package after failed install, failed boot marker, or manual rollback.

Minimum output:
- Rollback test works without network access.

### G8-F07 Runtime Apply Hooks

Required behavior:
- Apply resource updates through asset registry update path.
- Apply interpreted logic asset updates through logic runtime.
- Support Android optional assembly adapter if scoped.

Minimum output:
- Package can update resources and logic assets in validation environment.

## Target Effects

- Update packages are safe, verifiable, and rollback-capable.
- Active content is never corrupted by a failed package.
- Platform-specific payloads are respected.

## Explicit Non-Goals

- No live-ops backend service.
- No app store SDK integration.
- No iOS executable payload.
- No redesign of asset registry schema.

## AI Execution Rules

- Verify before install.
- Stage before activate.
- Keep previous known-good package.
- Log every rejection/activation/rollback decision.
- Treat Android assembly payload as isolated optional code.

## Completion Signal

Gate 8 is complete when valid packages install, invalid packages reject, package activation is atomic, and rollback is proven.
