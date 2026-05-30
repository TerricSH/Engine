# Gate 7: Mobile And Hot Update Contracts

## Purpose

Define mobile runtime and hot update contracts before building package installers. This gate separates C# strong typing from mobile-safe hot update constraints.

## Entry Sync Point

- Desktop C# scripting is stable.
- Registry-based assets are stable.

## Parallel Workstreams

1. Mobile Runtime Strategy
   - Owns Android/iOS runtime constraints, AOT rules, mobile script API subset, and validation harness.
2. Interpreted Logic Asset Model
   - Owns mobile-safe hot-updatable logic assets such as behavior graphs, state machines, skill graphs, quest/dialogue DSL, or AI behavior trees.
3. Hot Update Manifest
   - Owns package manifest fields: engine version, script API version, asset list, logic asset list, hashes, signatures, platform payloads, rollback metadata.

## Contracts To Freeze

- `MobileHotUpdate-v0`
- Mobile-safe script API subset
- Interpreted logic asset categories
- Hot update manifest schema

## Exit Condition

- `MobileHotUpdate-v0` is frozen.
- iOS has an AOT-safe path.
- Android optional C# assembly patching is scoped as platform-specific.

## Parallel Safety Notes

- iOS path must not rely on downloaded executable C# code.
- Android assembly patching remains optional and isolated.
- Hot update package implementation waits for this gate to close.
