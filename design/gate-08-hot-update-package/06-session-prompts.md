# Gate 8 Session Prompts

Gate 8 implements the package system. Every session must preserve atomic install and rollback behavior.

## Session 8A: Manifest Verification Owner

Goal: Parse and verify update manifests.

Owns:
- `crates/engine-hot-update` manifest/verify modules

Must not edit:
- asset registry schema
- script API schema

Expected output:
- version/hash/signature/platform checks

Validation:
- valid/invalid manifest tests

## Session 8B: Package Cache And Installer Owner

Goal: Implement staging, versioned cache, atomic activation, rollback.

Owns:
- cache/install/rollback modules

Must not edit:
- payload application logic beyond handoff interface

Expected output:
- staged package cannot corrupt active package
- rollback works without network

Validation:
- interrupted install test
- rollback test

## Session 8C: Runtime Apply Owner

Goal: Apply verified packages to assets and interpreted logic.

Owns:
- apply bridge to asset registry and logic runtime
- Android optional assembly adapter if scoped

Must not edit:
- manifest compatibility rules

Expected output:
- resources and logic assets update through existing runtime paths

Validation:
- sample package updates validation content
