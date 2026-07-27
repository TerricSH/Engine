# engine-core architecture

`engine-core` is the engine's composition root. It wires subsystem crates
together, owns their lifetime, and exposes a stable facade to applications. It
must not become the implementation home of those subsystems.

## Boundary rules

1. **Composition Root** — `EngineRuntimeBuilder` installs component, asset,
   render-extension, and debug-draw providers. Registration belongs here;
   subsystem behavior belongs in the subsystem crate.
2. **Facade** — public runtime operations are grouped below `runtime/`.
   Rendering submission and asset synchronization live in
   `runtime/rendering.rs`; additional runtime domains should follow the same
   pattern.
3. **Adapter** — wire-safe managed-script commands are translated to native
   subsystem operations below `script_commands/`. Adapters may depend on both
   `engine-script` and the target subsystem, but the subsystem must never depend
   on `engine-core`.
4. **Extension Registry** — optional crates register components and render/debug
   producers through registries instead of adding hard-coded branches to the
   frame loop.
5. **Single source of truth** — command validation stays with the command
   contract; runtime mutation has one adapter implementation. Compatibility
   feature aliases must not be used as internal gates.

The intended dependency direction is:

```text
application
    |
engine-core (composition + facade + adapters)
    |                 |
subsystem crates      engine-script contract
    |
engine-renderer backend abstraction
    |
backend crates (Vulkan, future backends)
```

## Feature model

Leaf features are the supported unit of conditional compilation:

| Leaf feature | Capability |
| --- | --- |
| `subsystem-animation` | animation runtime and extraction |
| `subsystem-audio` | audio components and asset types |
| `subsystem-navigation` | navigation and AI-agent runtime |
| `subsystem-ui` | retained UI runtime and rendering |
| `subsystem-physics` | physics world, queries, and character collision |
| `subsystem-gameplay` | gameplay state/input orchestration |
| `subsystem-terrain` | terrain streaming and rendering |
| `subsystem-scripting-csharp` | managed-script host and command contract |
| `backend-vulkan` | Vulkan backend construction |

`runtime-subsystems`, `gameplay`, and `terrain` remain compatibility aliases for
existing applications. New `#[cfg]` expressions inside `engine-core` must use
leaf features. Cross-subsystem code uses the exact conjunction it needs, such
as `subsystem-animation + subsystem-physics` for ragdolls.

CI checks every leaf independently and checks the cross-feature pairs that own
an adapter or integration path. Add a pair to `.github/scripts/ci.ps1` whenever
new code requires two optional capabilities.

## Source layout

```text
engine-core/src/
  lib.rs                    public types and composition root
  runtime/
    rendering.rs            rendering facade and asset synchronization
  script_commands/
    animation.rs            script-to-animation adapter
    ui.rs                   script-to-retained-UI adapter
  game_loop.rs              application loop orchestration
  terrain.rs                terrain integration facade
  tests.rs                  crate-root tests
```

When a block in `lib.rs` owns its own state transitions, validation, or domain
mapping, extract it behind a private module first. Create another crate only
when the code has a useful public API and can be compiled/tested without
`EngineRuntime`.
