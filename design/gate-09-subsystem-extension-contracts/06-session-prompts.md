# Gate 9 Session Prompts

Gate 9 creates extension surfaces. No real gameplay subsystem implementation should start here.

## Session 9A: Component And Asset Extension Owner

Goal: Implement component and asset type registration.

Owns:
- component extension registry
- asset type extension registry

Must not edit:
- real subsystem crates beyond dummy tests

Expected output:
- dummy component serializes
- dummy asset cooks/loads

Validation:
- dummy extension integration tests

## Session 9B: Editor And Script Extension Owner

Goal: Implement editor plugin and C# API extension surfaces.

Owns:
- editor plugin registry
- script API extension registry

Must not edit:
- core editor tool logic beyond extension host
- base `ScriptAPI-v0` semantics

Expected output:
- dummy panel and dummy binding register successfully

Validation:
- extension host tests

## Session 9C: Debug Draw And RendererInput Skinned-Items Owner

Goal: Implement renderer-facing extension surfaces.

Owns:
- debug draw surface
- `RenderExtensionRegistry` plus the `skinned_items: [SkinnedItem]` field on `RendererInput-v0` (minor bump to v0.2, per `FD-007`)

Must not edit:
- physics/animation implementation
- Vulkan backend internals except renderer input integration

Expected output:
- dummy debug draw provider renders via renderer path
- dummy skinned producer writes 1+ `SkinnedItem` into `RendererInput-v0` and consumer sees it without backend internal access

Validation:
- debug draw smoke test
- skinned-items minor-bump compatibility test (consumer that knows only v0.1 ignores `skinned_items` cleanly)
