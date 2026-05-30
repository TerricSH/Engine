# Gate 15: Runtime UI System

## Purpose

Add runtime UI for games, separate from editor UI. This gate provides canvas, layout, rendering, input, widgets, serialization, and C# callbacks.

## Entry Sync Point

- Renderer layers, asset registry, input routing, and ECS extension surfaces are stable.
- Gate 14 prefab composition is available if UI widgets need reusable templates.

## Parallel Workstreams

1. Canvas And Layout
   - Owns `crates/engine-ui`.
   - Adds canvas hierarchy, anchoring, sizing, layout constraints, screen-space coordinate handling, and UI transform model.
2. UI Rendering
   - Extracts UI quads/images/text into renderer commands without direct backend coupling.
   - Supports basic draw order, clipping, and atlas/texture references.
3. Input And Focus
   - Adds pointer hit testing, keyboard focus, mouse/touch routing, hover/pressed states, and input capture.
4. Core Widgets
   - Adds panel, image, text, button, checkbox/toggle, slider, and scroll view basics.
5. UI Scene Serialization And C#
   - Saves UI hierarchy through ECS/scene serialization.
   - C# APIs for button callbacks, setting text, toggling visibility, and reading widget values.

## Exit Condition

- Runtime UI scene renders over gameplay.
- Mouse/touch input triggers widget callbacks.
- UI scenes save/load and can be referenced by game state.
- UI does not depend on editor UI internals.

## Non-Goals

- Full UI editor, complex responsive breakpoints, localization system, data binding, and UI animation timeline.

## Parallel Safety Notes

- Runtime UI crate stays separate from editor UI implementation.
- UI rendering emits renderer input; it does not call backend APIs directly.
- UI input routing must not bypass platform input ownership.
