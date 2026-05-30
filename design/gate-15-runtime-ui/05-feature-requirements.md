# Gate 15 Feature Requirements And Execution Boundaries

## Gate Objective

Implement runtime game UI that is separate from editor UI and can render, receive input, serialize, and call into gameplay scripts.

## Required Features

### G15-F01 Runtime UI Crate

Required behavior:
- Implement `engine-ui` as a runtime subsystem separate from `engine-editor`.
- Register UI components through subsystem extension surfaces.

Minimum output:
- Runtime UI crate compiles and does not import editor UI internals.

### G15-F02 Canvas And Layout

Required behavior:
- Implement canvas root, coordinate conversion, scaling policy, anchors, sizes, and simple layout constraints.

Minimum output:
- UI elements position correctly at two or more window sizes.

### G15-F03 UI Rendering Extraction

Required behavior:
- Extract UI panels, images, text, clipping, and draw order into renderer input.
- Do not call backend APIs directly.

Minimum output:
- UI renders over gameplay scene.

### G15-F04 Text Rendering Path

Required behavior:
- Implement or integrate text shaping/layout/glyph cache at minimum scope.
- Support basic font asset references.

Minimum output:
- Text widget renders stable text in the validation scene.

### G15-F05 Input, Focus, And Capture

Required behavior:
- Implement pointer hit testing, hover, pressed state, keyboard focus, mouse/touch routing, and input capture.

Minimum output:
- Button click triggers callback and can block gameplay input when focused/captured.

### G15-F06 Core Widgets

Required behavior:
- Implement panel, image, text, button, toggle/checkbox, slider, and scroll view at agreed minimum scope.

Minimum output:
- Validation UI scene uses all core widgets.

### G15-F07 Serialization And C# Callbacks

Required behavior:
- Save/load UI hierarchy.
- Expose C# callbacks for common widget events.

Minimum output:
- UI scene round-trips and C# receives button/toggle/slider events.

## Target Effects

- Game UI can be authored and rendered separately from editor UI.
- UI can safely interact with gameplay through callbacks/events.
- UI input works across mouse/touch/keyboard at minimum scope.

## Explicit Non-Goals

- No full UI editor.
- No localization system.
- No complex responsive breakpoint system.
- No full data binding framework.
- No UI animation timeline.

## AI Execution Rules

- Runtime UI must not depend on editor UI.
- Render extraction must be backend-neutral.
- Input capture rules must be explicit.
- Keep text rendering isolated behind a module.

## Completion Signal

Gate 15 is complete when runtime UI renders, receives input, serializes, and triggers C# callbacks without backend/editor coupling.
