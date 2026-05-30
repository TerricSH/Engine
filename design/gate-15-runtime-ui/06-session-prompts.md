# Gate 15 Session Prompts

Gate 15 implements runtime UI. It must stay separate from editor UI.

## Session 15A: Canvas And Layout Owner

Goal: Implement canvas, coordinate spaces, anchors, sizing, and layout.

Owns:
- `crates/engine-ui` canvas/layout modules

Must not edit:
- editor UI internals

Expected output:
- responsive positioning at agreed minimum scope

Validation:
- layout tests and multi-size UI scene

## Session 15B: UI Render And Text Owner

Goal: Implement UI render extraction and text rendering path.

Owns:
- UI render extraction
- text shaping/glyph cache module

Must not edit:
- render backend internals except renderer input integration

Expected output:
- panels/images/text render over gameplay

Validation:
- UI render scene

## Session 15C: UI Input And Widgets Owner

Goal: Implement hit testing, focus/capture, widgets, serialization, and C# callbacks.

Owns:
- UI input/widgets/script modules

Must not edit:
- platform input internals beyond agreed events

Expected output:
- button/toggle/slider/scroll view interactions

Validation:
- callback and serialization tests
