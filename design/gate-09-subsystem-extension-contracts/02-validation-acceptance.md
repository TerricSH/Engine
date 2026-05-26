# Gate 9 Validation And Acceptance

## Gate Exit Principle

Gate 9 is accepted only when later subsystems can register components, assets, editor panels, script APIs, debug draw, and skinned-item producers (writing into `RendererInput-v0`) without modifying central core files.

## Required Results

- `SubsystemExtension-v0` is documented and implemented.
- Component extension registry exists.
- Asset type extension registry exists.
- Editor plugin surface exists.
- Script API extension surface exists.
- Debug draw surface exists.
- `RendererInput-v0` is bumped to v0.2 with `skinned_items: [SkinnedItem]` and a `RenderExtensionRegistry` entry point exists for animation to register a producer (per `FD-007`).

## Acceptance Criteria

- [ ] A dummy subsystem can register a component and serialize it without editing core scene parser code.
- [ ] A dummy subsystem can register an asset cooker/validator/loader without editing asset registry internals.
- [ ] A dummy subsystem can register an editor panel through the plugin surface.
- [ ] A dummy subsystem can register C# bindings through the script extension surface.
- [ ] Debug draw commands can be submitted without direct Vulkan calls.
- [ ] A dummy skinned producer writes `SkinnedItem` records into `RendererInput-v0.skinned_items` without animation code editing backend internals.

## Automated Checks

- `cargo fmt --check`
- `cargo check --workspace --features backend-vulkan,tooling-editor,subsystem-scripting-csharp`
- Extension registry unit tests.
- Dummy subsystem integration tests.
- Debug draw submission tests.

## Manual Validation

- Review that physics/animation can be planned as external users of the extension surfaces.
- Review that no central enum or monolithic parser must be edited for every subsystem.

## Blocking Conditions

- Physics or animation would need to edit ECS/asset/editor/script core files directly.
- Debug draw requires backend-specific calls.
- Skinned rendering requires animation to mutate Vulkan backend code or invent a parallel contract instead of using `RendererInput-v0.skinned_items`.

## Required Evidence

- Dummy subsystem test output.
- Extension API review note.
- List of frozen extension contracts.

## Exit Decision

- Gate owner:
- Date:
- Approved to proceed to Gate 10: yes/no

