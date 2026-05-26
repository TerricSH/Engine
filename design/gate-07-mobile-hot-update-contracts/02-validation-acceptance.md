# Gate 7 Validation And Acceptance

## Gate Exit Principle

Gate 7 is accepted only when mobile runtime constraints and hot update contracts are documented, versioned, and validated enough for package implementation to begin.

## Required Results

- Android and iOS runtime constraints are documented separately.
- iOS AOT-safe scripting path is defined.
- Android optional C# assembly patching is scoped as a platform-specific extension.
- Interpreted logic asset model is defined.
- `MobileHotUpdate-v0` manifest schema is frozen.
- Engine/script API version compatibility rules are defined.

## Acceptance Criteria

- [ ] Manifest includes engine version, script API version, asset list, logic asset list, hashes, signatures, platform payload fields, and rollback metadata.
- [ ] iOS path does not require downloaded executable C# code.
- [ ] Android assembly patch path can be disabled without affecting cross-platform updates.
- [ ] Interpreted logic assets have at least one concrete first target, such as behavior graph or state machine.
- [ ] Mobile script API subset is documented.

## Automated Checks

- Manifest schema validation tests.
- Compatibility check tests for matching and mismatched engine/script API versions.
- Logic asset schema parse tests.

## Manual Validation

- Review iOS path against AOT and dynamic-code restrictions.
- Review Android optional assembly update assumptions.
- Walk through a sample manifest and explain how each platform consumes it.

## Blocking Conditions

- Hot update design depends on iOS downloading executable code.
- Manifest cannot express platform-specific payloads.
- Engine/script API compatibility is undefined.

## Required Evidence

- Mobile runtime strategy note.
- `MobileHotUpdate-v0` schema example.
- Compatibility test output.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 8: yes/no

