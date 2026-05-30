# Gate 9: Subsystem Extension Contracts

## Purpose

Create plugin-style extension surfaces before physics, animation, UI, audio, and future gameplay systems start. This gate prevents later subsystems from editing the same ECS, asset, editor, and script core files.

## Entry Sync Point

- ECS is stable.
- Asset registry is stable.
- Editor inspector is stable.
- C# scripting is stable.
- Scene serialization is stable.

## Parallel Workstreams

1. Component Extension Registry
   - Defines how subsystem components register ECS storage, scene serialization, editor display, and script exposure.
2. Asset Type Extension Registry
   - Defines how subsystem asset types register cookers, validators, registry loaders, and hot update metadata.
3. Editor Plugin Surface
   - Defines how subsystem panels register component inspectors, debug views, and validation messages.
4. Script API Extension Surface
   - Defines how subsystem APIs add C# bindings without breaking `ScriptAPI-v0`.
5. Debug Draw Surface
   - Defines a renderer-independent path for colliders, skeletons, bounds, nav probes, and other tools.
6. Skinned Render Input Contract
   - Defines how animated/skinned meshes pass bone palettes or skinning data to renderer input without animation code editing backend internals.

## Contracts To Freeze

- `SubsystemExtension-v0`
- Debug draw surface
- Skinned render input contract
- Plugin registration rules

## Exit Condition

- `SubsystemExtension-v0` is frozen.
- Physics and animation can start in separate sessions without editing shared ECS/asset/editor/script core files.

## Parallel Safety Notes

- New subsystem fields must register through extension APIs.
- Avoid central enum growth unless it is explicitly versioned.
- Debug visualization never calls Vulkan directly.
