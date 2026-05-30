# Gate 7 Session Prompts

Gate 7 is contract/design heavy. Coding sessions should produce schemas, validators, and harnesses, not package installers.

## Session 7A: Mobile Runtime Profile Owner

Goal: Define Android/iOS/desktop runtime profiles.

Owns:
- mobile profile modules/docs
- profile validation tests

Must not edit:
- hot update installer code
- C# host internals except profile consumption hooks

Expected output:
- Desktop, Android, iOS profiles
- iOS AOT-safe constraints encoded

Validation:
- profile compatibility tests

## Session 7B: Interpreted Logic Asset Owner

Goal: Define first hot-updatable interpreted logic asset model.

Owns:
- logic asset schema/docs/tests

Must not edit:
- C# runtime semantics
- package installer

Expected output:
- typed behavior/state graph schema
- parse/validation tests

Validation:
- example logic asset parses and validates

## Session 7C: Hot Update Manifest Owner

Goal: Define `MobileHotUpdate-v0`.

Owns:
- manifest schema and compatibility tests

Must not edit:
- package install implementation

Expected output:
- manifest fields, version checks, platform payload rules

Validation:
- accept/reject compatibility matrix
