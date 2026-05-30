# Gate 9 Feature Requirements And Execution Boundaries

## Gate Objective

Create extension surfaces so future subsystems can register components, assets, editor panels, script APIs, debug draw, and renderer inputs without modifying core engine files.

## Required Features

### G9-F01 Component Extension Registry

Required behavior:
- Register component type ID, storage factory, serialization hooks, editor metadata, and script exposure metadata.
- Support versioned component schemas.

Minimum output:
- Dummy component can serialize, deserialize, and appear in editor metadata.

### G9-F02 Asset Type Extension Registry

Required behavior:
- Register asset type ID, source extensions, cooker, validator, loader, and hot update metadata.

Minimum output:
- Dummy asset type can cook, validate, and load through registry extension APIs.

### G9-F03 Editor Plugin Surface

Required behavior:
- Register panels, component inspectors, debug views, validation messages, and menu commands.

Minimum output:
- Dummy subsystem panel appears through plugin registration.

### G9-F04 Script API Extension Surface

Required behavior:
- Allow subsystems to add C# bindings without changing the base `ScriptAPI-v0`.
- Track API extension version and dependencies.

Minimum output:
- Dummy C# binding is registered and discoverable.

### G9-F05 Debug Draw Surface

Required behavior:
- Provide backend-independent debug draw submission for lines, shapes, labels, bounds, colliders, skeletons, and nav probes at agreed minimum scope.

Minimum output:
- Dummy debug provider renders through renderer debug path without direct Vulkan calls.

### G9-F06 RendererInput-v0 skinned items channel

Required behavior:
- Extend `RendererInput-v0` with `skinned_items: [SkinnedItem]` (per `FD-007`); this is a minor bump to v0.2 of the canonical Gate 3 contract, not a new `-v0` contract.
- Provide a `RenderExtensionRegistry` entry point that animation (Gate 10) can register a producer against; producers write `SkinnedItem` records into the frame's `RendererInput-v0`.
- Validation of palette/skeleton size and bone index range happens before the item is added; invalid items are dropped with a diagnostic and the rest of the frame still submits.

Minimum output:
- Dummy skinned producer feeds 1+ `SkinnedItem` into `RendererInput-v0` and the renderer consumes them without backend internal access.
- A negative test confirms a palette/skeleton size mismatch drops only that item.

## Target Effects

- Physics, animation, UI, audio, navigation, and later systems can start in parallel.
- Core ECS, serializer, asset registry, editor, script, and renderer files remain stable.

## Explicit Non-Goals

- No actual physics/animation/UI/audio/nav implementation.
- No dynamic plugin loading ABI.
- No broad reflection framework beyond metadata required here.

## AI Execution Rules

- Do not add central enums that every subsystem edits.
- Use deterministic registration order.
- Add dummy subsystem tests.
- Keep debug draw backend-independent.

## Completion Signal

Gate 9 is complete when dummy subsystem registration proves all extension surfaces and `SubsystemExtension-v0` is frozen.
