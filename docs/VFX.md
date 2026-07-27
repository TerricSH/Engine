# Particle and decal foundation

The runtime registers two scene components in every build:

- `engine.vfx.particle_emitter`
- `engine.vfx.decal`

Both use the normal component registry, strict scene loading, editor metadata,
and generic read/write script component bridge.

## Particle emitters

`ParticleEmitter` supports deterministic CPU and analytic GPU simulation.
Both modes support continuous emission and a one-shot burst, a hard particle
budget, random lifetime and speed ranges, a directional cone, acceleration,
angular velocity, linear start-to-end size and RGBA interpolation, exponential
drag, and deterministic turbulence. CPU particles are rebased during a
world-origin shift; GPU batches update their analytic origin instead.
For a non-looping emitter, `duration` limits emission; zero means unlimited.

Particles render as camera-facing quads through `RenderFrameInput::particle_batches`.
CPU mode performs per-particle visibility checks and submits a compact instance
stream. GPU mode submits a 128-byte simulation block and a bounded instance
range; Vulkan and DX12 evaluate particles from `InstanceID`. Backends that do
not advertise GPU particle support receive the same deterministic CPU-expanded
instances automatically. Both modes reuse material alpha modes, backend asset
synchronization, frustum/layer culling, sorted transparency, and weighted
blended OIT.
The runtime supplies:

- mesh: `mesh-vfx-quad`
- blended double-sided material: `mat-vfx-default`

Projects can replace either asset on each emitter. A project material can add
a smoke, spark, or dust texture without changing the emitter. Materials using
the portable `Additive` surface mode select a dedicated additive particle PSO
on both Vulkan and DX12.

Only authored emitter configuration is saved. Live particles, random state,
elapsed time, and emission accumulators restart after a scene or save reload;
this avoids serializing large transient arrays and makes restart behavior
deterministic.

## Decals

`Decal` renders a local XY quad with local +Z as its surface normal. Its
`normal_bias` moves the quad slightly away from the receiving surface to
reduce z-fighting. `size` controls width and height. `lifetime == 0` means
permanent; a positive lifetime removes the decal from extraction after it
expires.

For a bullet mark or scorch mark, place the entity at the impact point and
rotate local +Z to the hit normal. Use a project-authored blended or masked
material and disable shadow casting (the extractor does this automatically).

## Current limits

This is the production-facing foundation, not a full effects editor. GPU mode
uses analytic vertex evaluation rather than a stateful compute simulation. It
does not yet provide arbitrary color/alpha curves, collision or sub-emitters,
soft-particle depth fading, or volume-projected decals with receiver masks and
normal blending. Decals are surface-aligned meshes, so a single decal does not
wrap around corners.
