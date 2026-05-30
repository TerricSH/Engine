# Lighting System

This document is the authoritative design for how the engine **shades and lights** pixels. It is cross-cutting because lighting touches `RHI-v0`, `RendererInput-v0`, `ECSScene-v0`, shaders, asset cooking, performance budgets, and the mobile/desktop split.

If a gate document and this document disagree, this document wins; update the gate document through the contract change workflow.

## Scope

In scope (v0):

- Lighting model (BRDF / shading equation).
- Color space, light units, exposure, tone-mapping pipeline.
- Light kinds and their parameters.
- Shading pipeline (forward / forward+ / clustered) and per-gate evolution.
- Shadow algorithm and per-gate evolution.
- Ambient / environment lighting model.
- Per-gate ownership and acceptance.

Explicitly out of scope (v0):

- Real-time global illumination (SSGI, Lumen-style, voxel GI). Future RFC if needed.
- Screen-space reflections (SSR).
- Refraction / sub-surface scattering / cloth-specific BRDFs.
- Path tracing or ray-traced shadows.
- Lightmap baker / probe baker tooling. Probes themselves may be authored, baking is deferred.

## Foundation Decisions Applied

| Decision | Applied as |
|---|---|
| `FD-002` Engine threading model | All light extraction runs on the main thread (ECS read), is double-buffered into the render thread's `RendererInput-v0`. The render thread never reads the ECS world. |
| `FD-003` iOS graphics backend | Every lighting/shadow feature ships with a documented MoltenVK fallback; features that cannot run on the MoltenVK feature subset are not allowed in the mobile path. |
| `FD-004` Shader toolchain | All lighting shaders are authored in GLSL, compiled with `shaderc`/`glslang` to SPIR-V, reflected with `spirv-reflect`. No HLSL. |
| `FD-005` Mobile budget reporting timing | From Gate 5 onward, every gate that touches lighting must dual-report desktop and mobile-simulator numbers for its required lit fixture. |
| `FD-008` IO and async runtime model | Cubemap / IBL prefilter / shadow atlas allocation runs through the synchronous IO pool; no async runtime is allowed in the shading path. |
| `FD-010` Cargo feature flag taxonomy | Advanced lighting features ship behind `subsystem-lighting-*` features (e.g. `subsystem-lighting-csm`, `subsystem-lighting-ibl`, `subsystem-lighting-cluster`). The core forward path is always on. |
| `FD-014` Logging and tracing | Lighting and shadow passes emit `tracing` spans (`shadow.csm.cascade_{i}`, `lighting.cluster_assign`, `lighting.ibl.prefilter`) for the profiler; no parallel logging. |

## Shading Model (FD-026)

The engine uses **PBR Metallic-Roughness** as the only shading model in v0:

- BRDF: GGX (Trowbridge-Reitz) specular + Lambert diffuse, energy-conserving multi-scattering compensation.
- Material parameters: `base_color` (sRGB texture or linear color), `metallic` (0..1), `roughness` (0..1), `normal` (tangent space), `occlusion` (0..1), `emissive` (linear HDR), `alpha_mode` (Opaque / Mask / Blend), `alpha_cutoff`.
- Alternate models (toon, anisotropic, cloth) are **not** v0; if added later, they live behind a new material type and a `subsystem-lighting-toon` style feature, never as a `bool` toggle on the default material.

Color and HDR pipeline (frozen):

- Working color space: **linear sRGB**.
- Texture sampling: sRGB-encoded color textures use the sRGB sampler view; normal / data textures use the linear sampler view. The material descriptor's `color_space` field is authoritative.
- HDR offscreen target: `R16G16B16A16_SFLOAT` for the lighting pass.
- Tone-mapping: **ACES (Narkowicz fitted)** in v0. Reinhard is allowed as a debug option.
- Display target: gamma-correct via the backend's swapchain sRGB attachment; no manual `pow(color, 1/2.2)` in user shaders.
- Exposure: physical, in EV. The camera component carries `aperture`, `shutter_speed`, `iso`, plus an auto-exposure override; the shading pass multiplies by `2^(EV100 - 16)` before tone-mapping.

Light units (frozen):

| Light kind | Unit | Notes |
|---|---|---|
| Directional | **lux** | Stored in `LightItem.intensity` as physical lux. Sun ~ 100 000 lux at noon. |
| Point / Spot | **lumens** | Stored in `LightItem.intensity`. The shader converts to radiant intensity (cd/sr) for spot, isotropic for point. |
| Area (deferred) | **lumens** | Not in v0 contract. |
| Emissive material | **nits** (cd/m²) | Material `emissive` field is in linear nits before exposure. |

The conversion table lives in `engine-renderer::lighting::units` and is the only place that touches unit conversion.

## Shading Pipeline (FD-027)

The shading pipeline evolves in three steps. Each step is a separate gate; do **not** ship a step before the previous one is accepted.

| Step | Gate | Algorithm | Notes |
|---|---|---|---|
| Step 0: minimum forward | **Gate 3** | Single-pass forward. At most one directional light + at most four point/spot lights per draw, evaluated in the fragment shader. | Acceptance: `forward-min` fixture renders a directional + 2 point lights on a textured PBR sphere. |
| Step 1: Forward+ (cluster light list) | **Gate 10 or 11** | CPU builds a per-view cluster grid; light-to-cluster assignment runs on the main thread, results live in a storage buffer the render thread consumes. Per-pixel light loop reads the cluster's light index list. Hard cap: 256 lights per view, 16 lights per cluster. | Feature flag: `subsystem-lighting-cluster`. Mobile fallback: clipped to step 0 if the device does not support storage buffers in fragment. |
| Step 2: Clustered Forward (GPU cluster build) | **Optional, future gate (post Gate 19)** | Cluster build moves to compute shader. | Not in v0. |

The renderer must never branch between forward and deferred at runtime; v0 is forward-only. A future Gate 20+ may introduce a deferred path as a separate `backend-renderer-deferred` feature.

## Shadow Algorithm (FD-028)

Shadows evolve in three steps, owned by specific gates:

| Step | Gate | Algorithm | Notes |
|---|---|---|---|
| Step 0: single directional shadow map | **Gate 3** | One 2048×2048 R32_SFLOAT shadow map for the primary directional light, fixed orthographic projection, 1×1 PCF. | Point/spot shadows are `Off` in Gate 3 even if `shadow_mode` requests them; the renderer downgrades and emits a diagnostic. |
| Step 1: CSM (cascaded shadow maps) for directional | **Gate 10 or 11** | 3 or 4 cascades, PSSM split (lambda configurable, default 0.5), 3×3 PCF, slope-scaled depth bias. | Feature flag: `subsystem-lighting-csm`. Cascade count is an `OFQ-007` open question for the owning gate. |
| Step 2: cube-map shadows for point, perspective for spot | **Gate 10 or 11** | One cube-map per shadow-casting point light (1024² per face), one perspective shadow map per shadow-casting spot light. Hard cap: 4 shadow-casting point/spot per view. | Same feature gate as step 1. |

Soft shadow techniques (PCSS, contact-hardening, ray-traced shadows) are **out of scope** for v0.

Authoring rules:

- A light's `shadow_mode` may request more than what the current build supports; the renderer downgrades to the highest available mode and emits a `Diagnostic` once per asset, never per frame.
- `cast_shadows: false` drawables are excluded from every shadow pass regardless of light kind.

## Ambient and Environment Lighting

| Step | Gate | Source |
|---|---|---|
| Step 0: constant ambient | **Gate 3** | `SceneSettings.ambient` field: linear-RGB constant added to diffuse. Suitable for first lit fixture, not for shipped quality. |
| Step 1: image-based lighting (IBL) | **Gate 10 or 11** | One environment cubemap per scene; prefiltered specular cubemap (5 mip levels) + irradiance cubemap (8×8 SH-equivalent), built offline by the cook step. Sky shader samples the raw env cube. | Feature flag: `subsystem-lighting-ibl`.
| Step 2: light probes / SH grids | **Not v0** | Tracked under `OFQ-008`. |

Skybox rendering is part of the lighting/environment surface, not a separate "sky" gate. The sky material is a normal `Material` whose vertex shader generates a full-screen cube and whose fragment samples the env cubemap with the camera direction.

## Light Culling Strategy

| Step | CPU culling | GPU culling |
|---|---|---|
| Gate 3 | All lights submitted; no cluster build. | None. |
| Forward+ step (Gate 10/11) | Frustum cull per view on the main thread, then build the cluster light list on the main thread. Hard caps: 256 lights / view, 16 lights / cluster. | Light index buffer is read-only on the GPU. |
| Cluster GPU build | Not v0. | Tracked as `OFQ-009`. |

## Per-Gate Ownership

| Gate | Responsibility |
|---|---|
| Gate 3 | Owns the forward-min lighting + single directional shadow + constant ambient. Freezes `LightItem` and `engine.light` field shape (in collaboration with Gate 4 for the ECS component). Sets up the HDR offscreen target and ACES tone-mapping. |
| Gate 4 | Owns the `engine.light` ECS component schema and extraction into `RendererInput-v0.lights`. |
| Gate 5 | Owns texture / cubemap importer for env maps; cooks normal-map convention (OpenGL +Y up by default). |
| Gate 10 or 11 | Owns Forward+, CSM, IBL behind `subsystem-lighting-*` features. The exact split between Gate 10 and Gate 11 is recorded in each gate's `01-code-architecture.md`. |
| Gate 17 | Owns editor inspectors for lights and environment, debug visualizations (cascade overlay, cluster heatmap), and tone-mapping debug views. |
| Gate 19 | Owns the mobile vs desktop preset matrix in the release pipeline (e.g. mobile defaults to CSM=3 cascades / no IBL prefilter mips above level 3). |

## Mobile / Desktop Subset

The mobile player runs **forward + single directional shadow + constant ambient** as the guaranteed baseline. Forward+, CSM, and IBL are allowed on mobile **only** when:

- `BackendCapabilities` reports storage buffer support in fragment shaders;
- the per-frame budget in [performance-budgets.md](performance-budgets.md) is met on the mobile simulator profile (`OFQ-002`).

If either check fails at app startup, the renderer falls back to step 0 and emits a one-time diagnostic. The renderer never silently disables shadows for individual lights once a scene starts.

## Open Questions

These are tracked in [foundation-decisions.md](foundation-decisions.md) under `OFQ-###`:

- `OFQ-007` CSM cascade count and split scheme (Gate 10/11 to resolve).
- `OFQ-008` Light probe / SH grid baking pipeline (post-v0 unless Gate 11 promotes it).
- `OFQ-009` GPU cluster build (post-v0).
- `OFQ-010` Real-time global illumination (post-v0; future RFC).

## Contract Change Workflow

Any change to the shading model, color space, units, tone-mapping curve, or default shadow algorithm requires:

1. An RFC against this file, the relevant FD entries, and `data-schema-contracts.md`.
2. Updates to every gate that references the changed area, with migration notes in each gate's `02-validation-acceptance.md`.
3. A re-baseline of [performance-budgets.md](performance-budgets.md) on both desktop and mobile simulator profiles.

This is the same workflow as the one defined in [compatibility-error-handling.md](compatibility-error-handling.md).
