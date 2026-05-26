# Gate 15 Test Plan

## Test Strategy

Gate 15 tests prove runtime UI renders, lays out, receives input, serializes, and triggers callbacks without depending on editor UI internals.

## Feature Test Cases

| Feature | Test Case | Type | Expected Result |
|---|---|---|---|
| G15-F01 Runtime UI Crate | Compile without editor UI | Compile | Runtime UI has no editor dependency |
| G15-F02 Canvas And Layout | Multi-resolution layout test | Unit/Visual | Elements remain positioned correctly |
| G15-F03 UI Rendering Extraction | Render panel/image/text | Integration | UI draws over gameplay |
| G15-F04 Text Rendering Path | Render text widget | Visual | Text appears with stable layout |
| G15-F05 Input, Focus, And Capture | Button captures click | Integration | Callback fires and gameplay input is blocked if captured |
| G15-F06 Core Widgets | Widget validation scene | Integration | All basic widgets render and respond |
| G15-F07 Serialization And C# Callbacks | Save/load UI and trigger C# event | Integration | UI persists and callback works |

## Gate Integration Tests

1. Runtime HUD scene.
2. Mouse/touch input scene.
3. UI save/load scene.
4. UI plus gameplay input capture test.

## Failure Criteria

- Runtime UI imports editor UI internals.
- UI calls renderer backend APIs directly.
- UI input cannot capture or route events deterministically.

## Test Fixtures

- `scenes/gate15_hud.scene`: HUD with panel, text, image, button, toggle, slider, scroll view.
- `scenes/gate15_input_capture.scene`: gameplay action and UI button overlap.
- Window sizes: 1280x720, 1920x1080, 800x600.
- Deterministic input script with click, touch, and key events.

## Executable Integration Cases

### IT-G15-01 Layout Snapshot

Steps:
1. Load HUD scene at each target window size.
2. Compute layout.
3. Dump widget rectangles.

Expected:
- Rects match stored layout snapshots within tolerance.
- No widget has negative size or NaN coordinates.

Evidence:
- Layout snapshot JSON for each resolution.

### IT-G15-02 Input Capture

Steps:
1. Load input capture scene.
2. Click UI button at fixed coordinates.
3. Press gameplay action key while UI capture is active.

Expected:
- Button callback count increments once.
- Gameplay action count remains zero while UI captures input.

Evidence:
- Event trace log.

### IT-G15-03 UI Render And Serialization

Steps:
1. Render HUD scene.
2. Save UI hierarchy.
3. Reload and render again.

Expected:
- UI appears in both renders.
- Serialized UI component count and widget properties match.

Evidence:
- Before/after screenshots.
- UI scene diff.
