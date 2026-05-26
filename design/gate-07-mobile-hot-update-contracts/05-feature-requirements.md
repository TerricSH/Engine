# Gate 7 Feature Requirements And Execution Boundaries

## Gate Objective

Define mobile runtime profiles and hot update contracts before any package implementation exists. This gate must settle what is cross-platform, what is Android-only, and what is iOS-safe.

## Required Features

### G7-F01 Platform Runtime Profiles

Required behavior:
- Define `DesktopProfile`, `AndroidProfile`, and `IosProfile` or equivalent runtime profile data.
- Capture AOT/JIT capability, dynamic assembly support, resource update support, interpreted logic support, and platform policy notes.
- Make profiles queryable by tooling and update validation.

Minimum output:
- Profiles documented and testable.
- iOS profile explicitly disallows downloaded executable C# payloads.

Do not overbuild:
- No real mobile deployment pipeline.
- No package installer.

### G7-F02 Mobile Script API Subset

Required behavior:
- Define the mobile-safe profile of `ScriptAPI-v0` supported by mobile builds.
- Include compatibility versioning expressed as a `script_api_version_range` constraint inside `MobileHotUpdate-v0`, not as a separate `-v0` contract.
- Document unsupported reflection/dynamic-loading patterns.

Minimum output:
- A documented mobile profile (subset) of `ScriptAPI-v0` with the unsupported patterns listed and a worked compatibility-range example.

Do not overbuild:
- No mobile script runtime implementation beyond contract/harness needs.

### G7-F03 Interpreted Logic Asset Contract

Required behavior:
- Define the first mobile-safe hot-updatable logic asset type.
- Include schema version, typed parameters, node/state list, transitions, and asset references.
- Keep it deterministic and narrow.

Minimum output:
- Example logic asset schema and parse/validation tests.

Do not overbuild:
- No general weakly typed scripting language.
- No visual graph editor.

### G7-F04 MobileHotUpdate-v0 Manifest

Required behavior:
- Define manifest fields for engine version, script API version, content schema, asset payloads, logic payloads, platform payloads, hashes, signatures, dependencies, and rollback metadata.
- Define compatibility rejection rules.

Minimum output:
- Example manifest and schema validation tests.

### G7-F05 Android Optional Assembly Policy

Required behavior:
- Define Android C# assembly payload as optional and platform-specific.
- Define how it is disabled and how shared gameplay avoids depending on it.

Minimum output:
- Android assembly payload rules are documented.

## Target Effects

- Mobile constraints are clear before package code begins.
- iOS-safe path is viable without dynamic code payloads.
- Android optional behavior does not infect cross-platform assumptions.

## Explicit Non-Goals

- No package download/install/rollback.
- No real app store build pipeline.
- No iOS downloaded C# assemblies.
- No broad script replacement language.

## AI Execution Rules

- Treat Apple/iOS policy as an architecture constraint.
- Keep Android-only assembly patching optional.
- Do not alter `ScriptAPI-v0` without versioned compatibility design.
- Keep interpreted logic typed and narrow.

## Completion Signal

Gate 7 is complete when platform profiles, mobile script subset, interpreted logic asset schema, and `MobileHotUpdate-v0` manifest are documented and validated.
