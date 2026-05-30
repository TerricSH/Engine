# Gate 12 Session Prompts

Gate 12 adds character movement authority. Do not bypass physics or animation public APIs.

## Session 12A: Controller Runtime Owner

Goal: Implement controller component, movement commands, grounding, slope/step handling, and movement modes.

Owns:
- `crates/engine-character` or agreed character module

Must not edit:
- physics backend internals
- animation core internals

Expected output:
- walking/running/jumping/falling/landing
- authoritative movement state

Validation:
- controller validation scene

## Session 12B: Physics-Animation Sync Owner

Goal: Connect controller state to physics queries and locomotion animation parameters.

Owns:
- sync systems and locomotion parameter mapping

Must not edit:
- physics simulation internals
- animation state core beyond public APIs

Expected output:
- animation transitions driven by controller state

Validation:
- idle/walk/run/jump/fall/land visual test

## Session 12C: C# And Editor Owner

Goal: Expose character APIs and editor fields.

Owns:
- C# character bindings
- editor character inspector/debug panel

Must not edit:
- controller movement internals unless public API needs small additions

Expected output:
- C# movement sample
- controller debug draw

Validation:
- C# movement script test
