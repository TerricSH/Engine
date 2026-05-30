# Data Schema Contracts

This document defines field-level logical schemas for the contracts that later gates depend on. The exact Rust structs may evolve during implementation, but serialized fields, identity rules, validation rules, and compatibility behavior must match these contracts once the owning gate freezes them.

## Common Field Types

| Type | Representation | Rules |
|---|---|---|
| `SchemaVersion` | `{ major: u16, minor: u16, patch: u16 }` or semver string in external manifests | Major changes are incompatible unless a migration exists; minor changes may add optional/defaulted fields; patch changes do not alter schema. |
| `EngineVersion` | semver string | Must be recorded in scene, registry, package, and release metadata. |
| `ContractVersion` | semver string with contract name | Document/header form uses the major-only short name (e.g. `RHI-v0`, `RendererInput-v0`); the serialized field uses the full semver form (e.g. `RHI-v0.1.0`, `RendererInput-v0.1.0`, `ECSScene-v0.1.0`). |
| `PersistentId` | UUID string, preferably UUIDv7 for new authored content | Stable across save/load and hot reload. Runtime entity handles must not be serialized as persistent identity. |
| `AssetId` | UUID string plus optional human-readable logical path | The UUID is authoritative; paths are diagnostics and editor hints. |
| `ComponentTypeId` | reverse-DNS or crate-qualified string, e.g. `engine.transform` | Stable name for serialization and editor/plugin lookup. |
| `PropertyPath` | dotted path with array indexes, e.g. `Transform.local.position.x` | Used by prefab overrides, editor diffs, and diagnostics. |
| `Hash` | binary form: `[u8; 32]` (raw SHA-256 digest, little-endian on disk); JSON / manifest form: lowercase hex string of length 64; **SHA-256 only in v0** (no algorithm field). | Required for cooked artifacts and hot-update payloads. |
| `Signature` | `{ algorithm, key_id, value }` | Required for installable update packages and release artifacts. |
| `DiagnosticSeverity` | enum `Info \| Warning \| Error \| Fatal` | `Error` blocks gate validation; `Fatal` blocks runtime mutation or activation. |
| `Diagnostic` | `{ code: String, severity: DiagnosticSeverity, message: String, source: Option<String>, fields: Map<String, Value>, related: [Diagnostic] }` | Full envelope and rules are defined in [compatibility-error-handling.md](compatibility-error-handling.md); every contract's validation surface emits this type. |

Authoring formats:

- Human-authored scenes and prefabs use RON during Gates 4-14 unless a gate explicitly chooses JSON for tooling.
- Manifests, package metadata, reports, and compatibility snapshots use deterministic JSON.
- Cooked runtime binaries use the bincode-based format defined under [Cooked Asset Binary Format](#cooked-asset-binary-format) (per `FD-006`); they are derived artifacts and must preserve the logical fields below. **Binary encoding is little-endian** (bincode default); all multi-byte integers, hashes, and float arrays follow that rule.

## Math And Coordinate Conventions

Per [foundation-decisions.md](foundation-decisions.md) `FD-030` (math library) and `FD-031` (coordinate system, units, NDC), the names used throughout this document resolve as:

| Name | Concrete type | Notes |
|---|---|---|
| `Vec2` / `Vec3` / `Vec4` | `glam::Vec2` / `Vec3` / `Vec4`; serialized as `[f32; N]` little-endian | meters for spatial fields (per FD-031) |
| `Quat` | `glam::Quat`; serialized as `[f32; 4]` in `(x, y, z, w)` order | unit length |
| `Mat3` / `Mat4` | `glam::Mat3` / `Mat4`, **column-major**; serialized as flat `[f32; 9]` / `[f32; 16]` in column order | world-from-local |
| `Affine3A` | `glam::Affine3A`; serialized as `Mat4` | optional storage optimization |
| `LinearRgb` | `[f32; 3]` (no alpha) or `[f32; 4]` (with alpha) | working-space color (FD-026); values **not** clamped to `[0, 1]` |
| `Srgb` | `[f32; 3]` or `[f32; 4]` | display-referred color; only used in editor inspector and at asset import |
| `AxisAlignedBox` | `{ min: Vec3, max: Vec3 }` | component-wise `min <= max` |
| `Rect` | `{ min: Vec2, max: Vec2 }` | normalized `[0, 1]` of the render target unless otherwise specified; `min.x < max.x` and `min.y < max.y`. |
| `ClearFlags` | enum `ColorAndDepth \| DepthOnly \| Nothing \| Skybox` | per-camera clear behavior (per `FD-034`); `Skybox` requires `SceneSettings.environment_map` (Gate 10/11). |
| `BlendMode` | enum `Replace \| AlphaBlend \| Additive` | overlay camera composition mode (per `FD-035`); shared with material / UI passes. |
| `RenderLayerId` | enum (registered names; v0 ships `Default`, `Transparent`, `UI`, `PostProcess`, `Debug`, plus 27 user-reserved slots) | bit index `0..32` into `engine.camera.render_layer_mask`. The human-readable string on `engine.renderable.render_layer` maps one-to-one to a bit index; the mapping is owned by `engine-scene` and frozen at Gate 4. |
| `FrameStats` | `{ visible_drawables: u32, culled_drawables: u32, visible_lights: u32, culled_lights: u32, draw_calls: u32, triangles: u64, gpu_frame_ms: f32 }` | renderer-populated per-frame counters (per `FD-036`); surfaced through `tracing` spans. |
| `Transform` | `{ position: Vec3, rotation: Quat, scale: Vec3 }` | local space unless documented otherwise |
| `BonePaletteLayout` | enum `Full4x4 { count: u32 }` or `Packed3x4 { count: u32 }`; v0 ships `Full4x4` only | tells the GPU skinning shader which matrix layout the palette uses; `Packed3x4` is a Gate 11 expansion |
| `PathHandle` | `u64` (generation-tagged: high 32 bits = generation, low 32 bits = slot) | opaque navmesh path handle; consumers must not parse the bits |
| `Value` (in `ComponentRecord.fields`) | tagged sum: `Bool(bool)`, `Int(i64)`, `UInt(u64)`, `Float32(f32)`, `Float64(f64)`, `Str(String)`, `Vec3(Vec3)`, `Quat(Quat)`, `Color(LinearRgb)`, `Asset(AssetId)`, `Entity(PersistentId)`, `Enum(String)`, `List([Value])`, `Map([(String, Value)])` | matches RON's tagged-enum syntax; binary form via bincode |
| `PlatformProfile` | enum: `WindowsX64`, `LinuxX64`, `MacosArm64`, `MacosX64`, `Android`, `Ios`; `"all"` in text-form fields means platform-independent | cooked artifact / package target |
| `AssetType` | enum: `Mesh`, `Texture`, `Material`, `Pipeline`, `Shader`, `CookedShader`, `Scene`, `Prefab`, `AnimationClip`, `Skeleton`, `Navmesh`, `BehaviorAsset`, `AudioClip`, `Font`, `ScriptAssembly` | stable; `Pipeline` and `CookedShader` were added by `FD-042` (cooked shader artifact + PSO-bound authoring asset). New asset kinds require a schema minor bump. |
| `AssetState` | enum: `Imported`, `Cooking`, `Cooked`, `Failed { reason: String }`, `Outdated` | editor and CI use this to gate scene load |

World handedness (right-handed), up axis (+Y), forward axis (-Z), NDC depth range (`[0, 1]`, Vulkan-style reverse-Z friendly), UV origin (top-left), linear unit (1.0 = 1 meter), angle unit (radians) are pinned by `FD-031`. All schema fields above follow that convention.

## RHI-v0

Owner: Gate 1. Consumers: Gate 2 and all renderer backends.

```text
AdapterInfo {
  backend: BackendKind,
  name: String,
  vendor_id: Option<u32>,
  device_id: Option<u32>,
  driver_version: Option<String>,
  capabilities: BackendCapabilities
}

BackendCapabilities {
  max_texture_dimension_2d: u32,
  max_color_attachments: u8,
  supports_swapchain: bool,
  supports_timestamps: bool,
  supports_debug_markers: bool,
  supported_shader_formats: [ShaderFormat],
  supported_surface_formats: [TextureFormat],
  limits: ResourceLimits
}

ResourceHandle {
  kind: Buffer | Texture | Shader | Pipeline | BindGroup | RenderPass | Surface,
  index: u32,
  generation: u32
}
```

Rules:

- Handles are opaque values with generation checks; stale handles return `RhiError::InvalidHandle`.
- Backend-specific handles must not appear in public descriptors or serialized data.
- Creation APIs return `Result<Handle, RhiError>` and destruction is idempotent only for handles still owned by the device.
- Surface loss, device loss, unsupported format, allocation failure, and validation failure are distinct error codes.

Minimum descriptor fields:

| Descriptor | Required fields |
|---|---|
| `DeviceDescriptor` | `adapter`, `required_features`, `required_limits`, `debug_label`, `validation_mode` |
| `SurfaceDescriptor` | `window_handle`, `width`, `height`, `preferred_format`, `present_mode` |
| `BufferDescriptor` | `size_bytes`, `usage_flags`, `memory_hint`, `debug_label` |
| `TextureDescriptor` | `width`, `height`, `depth_or_layers`, `mip_levels`, `format`, `usage_flags`, `sample_count`, `debug_label` |
| `ShaderModuleDescriptor` | `format`, `entry_points`, `source_hash`, `debug_label` |
| `PipelineDescriptor` | `shader_modules`, `vertex_layout`, `bind_layouts`, `raster_state`, `depth_state`, `blend_state`, `render_targets`, `debug_label` |

## RendererInput-v0

Owner: Gate 3. Consumers: renderer backends, ECS extraction, UI, debug draw, profiling, animation runtime (skinned items added in v0.2 per `FD-007`).

```text
RenderFrameInput {
  contract_version: ContractVersion,
  frame_index: u64,
  views: [RenderView],
  drawables: [RenderableItem],
  skinned_items: [SkinnedItem],
  materials: [MaterialBinding],
  meshes: [MeshBinding],
  lights: [LightItem],
  debug_primitives: [DebugPrimitive],
  ui_batches: [UiBatch],
  stats_scope: Option<String>
}
```

Rules:

- Renderer input is immutable for a frame after submission.
- Items reference assets by `AssetId` or transient upload IDs, never source file paths.
- Missing assets are validation errors before render submission unless the caller explicitly enables debug placeholder rendering.
- Sort keys are deterministic and must not depend on pointer addresses.
- `skinned_items` is added at minor version `RendererInput-v0.2` (per `FD-007`); a producer that does not emit any skinned drawable submits an empty array, not a missing field.
- Bone palettes inside `SkinnedItem` are produced by animation extraction (Gate 10); renderer backends do not query animation state directly. A missing skeleton, palette/skeleton size mismatch, or invalid bone index drops the offending item with a diagnostic and does not abort the frame.
- `LightItem.kind` is the enum `LightKind = Directional | Point | Spot` (per `FD-026`). `Area` is **not** in v0; producers must not submit it.
- `LightItem.shadow_mode` is the enum `ShadowMode = Off | Hard | Soft` (per `FD-028`). A request that exceeds what the current build supports is downgraded by the renderer with a one-time diagnostic; the frame is not aborted.
- `LightItem.intensity` is **lux** for `Directional` and **lumens** for `Point`/`Spot` (per `FD-026`); the unit must match the kind or the item is dropped with a diagnostic.
- `LightItem.position` is ignored for `Directional`; `direction` is ignored for `Point`; `spot_angles` is required for `Spot` and must satisfy `0 <= inner <= outer <= PI/2` (radians).
- `LightItem.color` is in linear sRGB (per `FD-026`); values are not clamped (HDR allowed).
- The `render_options` field carries the per-frame tone-mapping selection (`Aces | Reinhard | None`, default `Aces` per `FD-026`) and an `exposure_ev100: f32` override; if absent the renderer derives exposure from `RenderView.camera` parameters.
- `RenderView.compose` and `RenderView.stack_order` follow the camera stack rules in `FD-035`: all `Base` views draw first sorted by `(camera.priority, stack_order, view_id)` and clear per their `ClearFlags`; all `Overlay` views draw afterward referencing `base_view_id`, sorted by `(stack_order, view_id)`, composited with `blend_mode`, and **never** clear color or depth. An overlay whose `base_view_id` is absent in the current frame is dropped with diagnostic `RV0007 OverlayBaseMissing` and the frame continues.
- `RenderableItem` / `LightItem` / `SkinnedItem` arrive **already culled** per `RenderView` (per `FD-036`); the renderer does not re-cull. When `RenderView.frustum` is present, debug builds may run a consistency check and emit `RV0009 CulledItemSubmitted` if a submitted item lies fully outside any plane.
- Per-camera `render_layer_mask` (Gate 4 `engine.camera`) is converted by extraction into the per-view filter: a `RenderableItem` reaches a `RenderView` only when `(1u32 << render_layer_bit(item.render_layer)) & camera.render_layer_mask != 0`.
- `engine.camera.render_target` must be `None` in v0 (see `OFQ-011`); a non-`None` value is rejected at extraction with diagnostic `SC0014 RenderTargetNotInV0` and the camera is treated as if it rendered to the swapchain.

Required fields:

| Type | Required fields |
|---|---|
| `RenderView` | `view_id`, `camera_entity: Option<PersistentId>`, `viewport: Rect` (pixel-space or normalized per-backend), `viewport_rect_normalized: Rect` (always normalized `[0,1]` of the render target; mirrors `engine.camera.viewport_rect`), `view_matrix`, `projection_matrix`, `clear_flags: ClearFlags`, `clear_color: LinearRgb`, `render_layer_mask: u32`, `msaa_samples: u8`, `compose: Base { clear: ClearFlags, clear_color: LinearRgb } \| Overlay { base_view_id: u32, blend_mode: BlendMode }`, `stack_order: i32`, `frustum: Option<[Vec4; 6]>` |
| `RenderableItem` | `entity: Option<PersistentId>`, `mesh: AssetId`, `material: AssetId`, `world_transform`, `bounds`, `render_layer`, `cast_shadows`, `sort_key` |
| `SkinnedItem` | `entity: Option<PersistentId>`, `mesh: AssetId`, `material: AssetId`, `skeleton: AssetId`, `bone_palette: [Mat4]`, `bone_palette_layout: BonePaletteLayout`, `world_transform: Mat4`, `bounds: AxisAlignedBox`, `render_layer: String`, `cast_shadows: bool`, `sort_key: u64` |
| `LightItem` | `entity: Option<PersistentId>`, `kind: LightKind`, `color: LinearRgb`, `intensity: f32`, `range: f32`, `position: Vec3`, `direction: Vec3`, `spot_angles: Option<{inner: f32, outer: f32}>`, `shadow_mode: ShadowMode` |
| `MaterialBinding` | `material_id: AssetId`, `pipeline: AssetId` (references a `Pipeline` asset per `FD-042`), `variant_key: u64` (bit-packed variant selector per `FD-040`; renderer ORs engine-reserved bits `SKINNED` / `INSTANCED` / `SHADOW_PASS` into the authored key before lookup), `textures: [TextureSlot]`, `uniforms: ParamBlock`, `pass_mask: u32` (bitmask of passes this material participates in), `transparency: Opaque \| Masked { cutoff: f32 } \| Blend`, `double_sided: bool` |
| `TextureSlot` | `binding: u32`, `texture: AssetId`, `sampler: AssetId`, `color_space: Linear \| Srgb` (per `FD-026`), `mip_bias: f32` |
| `ParamBlock` | `bytes: [u8]` (UBO payload bound at `set=1, binding=0` per `FD-041`, std140 layout, little-endian) plus a `layout_hash: Hash` computed as `sha256(set_index \|\| binding_index \|\| std140_layout_bytes)` from `spirv-reflect`; renderer rejects with `RV0010 ParamBlockLayoutMismatch` if `layout_hash` does not match the pipeline's expected layout |
| `MeshBinding` | `mesh_id: AssetId`, `vertex_layout: VertexLayout`, `index_format: U16 \| U32`, `submeshes: [Submesh]`, `bounds: AxisAlignedBox` |
| `Submesh` | `name: String`, `index_offset: u32`, `index_count: u32`, `material_slot: u32` |
| `VertexLayout` | `stride_bytes: u32`, `attributes: [{ semantic: Position \| Normal \| Tangent \| Uv0 \| Uv1 \| Color0 \| Joints0 \| Weights0, format: VertexAttributeFormat, offset_bytes: u32 }]` (every cooked mesh declares its layout up-front; renderer picks a compatible pipeline) |
| `DebugPrimitive` | `source_system: String`, `severity: DiagnosticSeverity`, `primitive_kind: Line { from: Vec3, to: Vec3 } \| Triangle { a: Vec3, b: Vec3, c: Vec3 } \| Sphere { center: Vec3, radius: f32 } \| Box { center: Vec3, half_extents: Vec3, rotation: Quat } \| Text { position: Vec3, text: String, size_px: f32 }`, `color: LinearRgb`, `lifetime_frames: u32` (0 = single frame) |
| `UiBatch` | `canvas_id: PersistentId`, `z_order: i32`, `clip_rect: { min: Vec2, max: Vec2 }`, `texture: Option<AssetId>`, `vertices: [{ position: Vec2, uv: Vec2, color: [u8; 4] }]`, `indices: [u32]`, `material: AssetId` (UI material; defaults to the engine's built-in unlit textured material) |

## ECSScene-v0

Owner: Gate 4. Consumers: editor, asset pipeline, scripting, prefab, gameplay systems, release QA.

```text
Scene {
  schema_version: SchemaVersion,
  engine_version: EngineVersion,
  scene_id: PersistentId,
  name: String,
  entities: [EntityRecord],
  scene_settings: SceneSettings,
  dependencies: [AssetId],
  diagnostics_policy: DiagnosticsPolicy
}

EntityRecord {
  persistent_id: PersistentId,
  parent: Option<PersistentId>,
  name: Option<String>,
  enabled: bool,
  components: Map<ComponentTypeId, ComponentRecord>
}

ComponentRecord {
  schema_version: SchemaVersion,
  enabled: bool,
  fields: Map<String, Value>
}

SceneSettings {
  active_camera: Option<PersistentId>,
  default_render_layer: String,
  fixed_timestep_seconds: f32,
  gravity: Option<Vec3>,
  ambient: LinearRgb,
  environment_map: Option<AssetId>,
  tone_mapping: ToneMapping
}
```

Validation:

- `persistent_id` values are unique within the scene.
- Parent references must point to existing entities and must not form cycles.
- `active_camera`, if present, must reference an enabled entity with an enabled `Camera` component.
- Component field validation runs before mutating the runtime world.
- Unknown component types are rejected by default; editor import tools may preserve them only when explicitly in compatibility mode.
- `SceneSettings.ambient` is a linear-RGB constant added to diffuse before tone-mapping; intended for the Gate 3 minimum lit fixture. When an `environment_map` is bound (Gate 10/11 IBL path), the ambient constant is added on top of the prefiltered irradiance contribution.
- `SceneSettings.environment_map`, if present, must reference a cubemap `AssetRecord` whose `asset_type` is `Texture` and whose cooked artifact carries the `cubemap` flag (per `FD-026`). The Gate 3 renderer ignores this field; Gate 10/11 IBL consumes it.
- `SceneSettings.tone_mapping` is the enum `ToneMapping = Aces | Reinhard | None` (per `FD-026`); defaults to `Aces` in v0.

Core component fields:

| Component | Required fields |
|---|---|
| `engine.name` | `value: String` |
| `engine.transform` | `local_position: Vec3`, `local_rotation: Quat`, `local_scale: Vec3`, `world_override: Option<Mat4>` |
| `engine.renderable` | `mesh: AssetId`, `material: AssetId`, `visible: bool`, `render_layer: String`, `cast_shadows: bool` |
| `engine.camera` | `projection: Perspective \| Orthographic`, `near: f32`, `far: f32`, `fov_y_or_size: f32`, `viewport_rect: Rect` (normalized `[0,1]`; default `{ min: (0,0), max: (1,1) }`), `render_layer_mask: u32` (default `0xFFFF_FFFF`), `clear_flags: ClearFlags` (default `ColorAndDepth`), `clear_color: LinearRgb`, `priority: i32`, `render_target: Option<AssetId>` (must be `None` in v0; see `OFQ-011`), `msaa_samples: u8` (`1` \| `2` \| `4` \| `8`; default `1`), `hdr_output: bool` (default `false`), `exposure: { aperture: f32, shutter_speed: f32, iso: f32, ev_compensation: f32 }` |
| `engine.light` | `kind: LightKind = Directional \| Point \| Spot`, `color: LinearRgb`, `intensity: f32` (lux for `Directional`, lumens for `Point`/`Spot`), `range: f32`, `spot_angles: Option<{inner: f32, outer: f32}>`, `shadow_mode: ShadowMode = Off \| Hard \| Soft` |
| `engine.bounds` | `local_min: Vec3`, `local_max: Vec3`, `update_policy` |

## AssetRegistry-v0

Owner: Gate 5. Consumers: ECS scenes, renderer extraction, scripts, editor, hot update, prefabs, audio, UI, release pipeline.

```text
AssetRegistry {
  schema_version: SchemaVersion,
  engine_version: EngineVersion,
  cook_profile: String,
  assets: [AssetRecord]
}

AssetRecord {
  asset_id: AssetId,
  logical_path: String,
  source_path: Option<String>,
  asset_type: AssetType,
  import_settings_hash: Hash,
  source_hash: Option<Hash>,
  cooked_artifacts: [CookedArtifact],
  dependencies: [AssetDependency],
  tags: [String],
  state: AssetState
}

CookedArtifact {
  artifact_id: String,
  platform: PlatformProfile,
  relative_path: String,
  content_hash: Hash,
  byte_size: u64,
  schema_version: SchemaVersion
}
```

Rules:

- `asset_id` is the only stable runtime identity.
- Cooked artifacts are immutable by content hash.
- Registry updates are atomic: build a new registry snapshot, validate it, then swap.
- Missing dependencies block package activation and scene load unless the scene is opened in editor repair mode.

## ScriptAPI-v0

Owner: Gate 5. Consumers: scripting, mobile profile validation, gameplay systems, UI/audio/character/nav APIs.

```text
ScriptApiContract {
  contract_version: ContractVersion,
  supported_platforms: [PlatformProfile],
  exposed_types: [ScriptType],
  callbacks: [CallbackDescriptor],
  serializable_field_types: [ScriptFieldType]
}

ScriptComponent {
  type_name: String,
  assembly_id: Option<AssetId>,
  enabled: bool,
  serialized_fields: Map<String, ScriptValue>,
  api_version_required: ContractVersion
}
```

Rules:

- Engine APIs exposed to C# are facade APIs; they do not expose raw ECS storage, backend renderer objects, or physics backend handles.
- Script exceptions are reported through diagnostics, mark the failing script instance as faulted for the current frame, and must not corrupt the world.
- Mobile-compatible APIs must be a documented subset of this contract.

## MobileHotUpdate-v0

Owner: Gate 7. Consumer: Gate 8 package installer and Gate 19 release pipeline.

```text
MobileHotUpdateManifest {
  manifest_version: SchemaVersion,
  package_id: String,
  package_version: String,
  engine_version_range: SemVerRange,
  script_api_version_range: SemVerRange,
  content_schema_versions: Map<String, SchemaVersion>,
  target_platforms: [PlatformProfile],
  payloads: [PayloadEntry],
  dependencies: [PackageDependency],
  rollback: RollbackMetadata,
  signatures: [Signature],
  created_by: String
}

PayloadEntry {
  payload_id: String,
  kind: asset_bundle | interpreted_logic | android_assembly | metadata,
  platform: PlatformProfile | "all",
  relative_path: String,
  hash: Hash,
  byte_size: u64,
  compressed: bool,
  required: bool
}
```

Rules:

- iOS payloads must not include `android_assembly` or any downloaded executable code.
- Compatibility validation happens before download when possible and before activation always.
- Hash validation covers the exact bytes to be activated.
- Signatures cover the manifest and payload hash list.

## PackageInstallState-v0

Owner: Gate 8. Consumers: hot update, asset registry, interpreted logic runtime, release QA.

```text
PackageState {
  active_package: Option<String>,
  previous_known_good: Option<String>,
  staged_package: Option<String>,
  state: Discovered | Downloading | Downloaded | Verified | Staged | Active | Rejected | FailedBoot | RolledBack,
  last_error: Option<Diagnostic>,
  activated_at_engine_version: Option<EngineVersion>
}
```

Rules:

- Activation is a metadata pointer switch; active files are never overwritten in place.
- Crash or interruption during activation recovers to `previous_known_good` or resumes from `Staged`.
- A package cannot become `Active` unless manifest, hashes, signatures, platform rules, and dependency checks pass.

## SubsystemExtension-v0

Owner: Gate 9. Consumers: physics, animation, UI, audio, navigation, editor, debug draw.

```text
SubsystemDescriptor {
  subsystem_id: String,
  contract_version: ContractVersion,
  component_types: [ComponentDescriptor],
  systems: [SystemDescriptor],
  debug_draw: [DebugDrawDescriptor],
  editor_panels: [EditorPanelDescriptor],
  script_bindings: [ScriptBindingDescriptor]
}
```

Rules:

- Subsystems register descriptors at startup; central engine enums must not be edited for every new subsystem.
- Registered component schemas must include migrations or default values before they are used in scenes or prefabs.
- Debug draw providers emit `DebugPrimitive` records; they do not call renderer backends directly.

## SkinnedRenderInput (folded into RendererInput-v0)

There is no standalone `SkinnedRenderInput-v0` contract (per `FD-007`). Skinned drawables are the `skinned_items: [SkinnedItem]` field on `RendererInput-v0`. The `SkinnedItem` schema and validation rules are documented under [RendererInput-v0](#rendererinput-v0) above.

## Physics/Animation-v0

Owner: Gate 10. Consumers: character controller, navigation/AI, editor, scripts.

```text
PhysicsWorldSettings {
  fixed_timestep_seconds: f32,
  gravity: Vec3,
  max_substeps: u8,
  solver_iterations: u8
}
```

Physics required component fields:

| Component | Required fields |
|---|---|
| `engine.physics.rigid_body` | `body_type: static \| kinematic \| dynamic`, `mass`, `linear_damping`, `angular_damping`, `gravity_scale`, `ccd_enabled`, `lock_translation`, `lock_rotation` |
| `engine.physics.collider` | `shape: box \| sphere \| capsule \| mesh`, `shape_params`, `local_offset`, `is_trigger`, `material: AssetId \| inline`, `collision_layer`, `collision_mask` |
| `engine.physics.material` | `friction`, `restitution`, `friction_combine`, `restitution_combine` |

Animation required component fields:

| Component | Required fields |
|---|---|
| `engine.animation.skeleton` | `skeleton_asset: AssetId`, `bone_count`, `root_bone_index` |
| `engine.animation.player` | `clip: AssetId`, `time_seconds`, `speed`, `looping`, `paused`, `weight` |
| `engine.animation.skinned_mesh_binding` | `skeleton_entity: PersistentId`, `mesh: AssetId`, `material: AssetId`, `bone_remap: Option<[u16]>` |

Query and event surfaces:

| Type | Required fields |
|---|---|
| `RaycastQuery` | `origin: Vec3`, `direction: Vec3`, `max_distance`, `layer_mask`, `query_flags` |
| `OverlapQuery` | `shape`, `transform`, `layer_mask`, `query_flags` |
| `SweepQuery` | `shape`, `start_transform`, `motion: Vec3`, `layer_mask`, `query_flags` |
| `CollisionEvent` | `kind: enter \| stay \| exit \| trigger`, `entity_a: PersistentId`, `entity_b: PersistentId`, `contact_points`, `normal`, `frame_index` |

Rules:

- Backend body, collider, skeleton, or pose handles are never serialized or returned to scripts; only public component fields and event records cross the boundary.
- Physics writes transforms only at fixed-step boundaries; mid-step script mutations are queued or rejected with a diagnostic.
- Animation evaluation feeds bone palettes into `RendererInput-v0.skinned_items` (per `FD-007`); the renderer does not read animation components directly.
- Event streams and query snapshots are valid only for the frame they are read; consumers must copy data they need to retain.

## CharacterController-v0

Owner: Gate 12. Consumers: AI agents, gameplay scripts, animation parameter feed, editor.

```text
CharacterControllerComponent {
  capsule_radius: f32,
  capsule_height: f32,
  slope_limit_degrees: f32,
  step_offset: f32,
  skin_offset: f32,
  walk_speed: f32,
  run_speed: f32,
  jump_impulse_or_height: f32,
  air_control: f32,
  gravity_scale: f32,
  movement_mode: grounded_idle | grounded_locomotion | jump_start | falling | landing,
  grounded: bool,
  ground_normal: Vec3,
  current_velocity: Vec3
}

CharacterMoveCommand {
  desired_direction: Vec3,
  desired_speed: f32,
  jump_requested: bool,
  mode_override: Option<MovementMode>,
  source: input | script | ai
}

CharacterMoveState {
  grounded: bool,
  movement_mode: MovementMode,
  current_speed: f32,
  vertical_velocity: f32,
  direction: Vec3,
  jump_event: Option<JumpEvent>,
  land_event: Option<LandEvent>
}
```

Rules:

- The character controller is the sole writer of transforms for entities that have a `CharacterControllerComponent`; physics resolves collision through public queries, animation reads movement state, and scripts/AI submit commands.
- Commands are consumed during the controller update; queued commands do not persist across frames.
- `CharacterMoveState` is a per-frame snapshot consumed by the animation parameter feed; it is not serialized.
- Movement mode transitions are deterministic given the same commands and physics queries.

## NavAI-v0

Owner: Gate 13. Consumers: gameplay framework, behavior runtime, editor diagnostics, scripts.

```text
NavmeshAsset {
  asset_id: AssetId,
  schema_version: SchemaVersion,
  agent_radius: f32,
  agent_height: f32,
  cell_size: f32,
  cell_height: f32,
  walkable_slope_degrees: f32,
  tiles: [NavmeshTile]
}
```

AI agent and behavior required fields:

| Type | Required fields |
|---|---|
| `engine.nav.agent` | `navmesh: AssetId`, `agent_radius`, `agent_height`, `target: Option<Vec3 \| PersistentId>`, `current_path: Option<PathHandle>`, `path_status: idle \| computing \| following \| arrived \| failed \| stopped`, `stopping_distance`, `movement_speed`, `controller_entity: PersistentId` |
| `PathRequest` | `agent: PersistentId`, `start: Vec3`, `goal: Vec3 \| PersistentId`, `corridor_radius`, `optional_constraints` |
| `PathResult` | `request_id`, `status: success \| partial \| failed`, `waypoints: [Vec3]`, `diagnostic: Option<Diagnostic>` |
| `BehaviorAssetRef` | `asset_id: AssetId`, `schema_version: SchemaVersion`, `entry_state_or_root_node: String` |
| `BlackboardEntry` | `key: String`, `type: bool \| i64 \| f64 \| vec3 \| entity \| asset \| string`, `value: Value` |

Rules:

- Agents request movement from the Gate 12 character controller and must not write transforms directly.
- Failed or stopped path status enters a diagnostic state that is observable from scripts and the editor; the agent does not teleport or skip path validation.
- Behavior runtime tick is deterministic given the same blackboard state and inputs.
- Navmesh assets are immutable at runtime in Gate 13; dynamic navmesh updates require a future schema change.

## Prefab-v0

Owner: Gate 14. Consumers: editor, scene serialization, gameplay framework, hot update, release QA.

```text
PrefabAsset {
  schema_version: SchemaVersion,
  prefab_id: AssetId,
  name: String,
  root_entities: [PrefabEntity],
  child_prefabs: [ChildPrefabRef],
  dependencies: [AssetId],
  validation: PrefabValidationMetadata
}

PrefabEntity {
  stable_path: String,
  parent_path: Option<String>,
  enabled: bool,
  components: Map<ComponentTypeId, ComponentRecord>
}

PrefabInstance {
  instance_id: PersistentId,
  source_prefab: AssetId,
  source_version: SchemaVersion,
  root_entity: PersistentId,
  overrides: [PrefabOverride]
}

PrefabOverride {
  target_path: String,
  component_type: Option<ComponentTypeId>,
  property_path: PropertyPath,
  operation: set | add | remove | enable | disable,
  value: Option<Value>
}
```

Validation:

- Nested prefabs must not form cycles.
- Overrides that target missing paths are preserved as unresolved in editor mode and rejected in runtime/package validation.
- Apply/revert uses explicit override records, not implicit diffing of runtime memory.

## RuntimeUI-v0

Owner: Gate 15. Consumers: renderer input, scripting, gameplay framework, editor tooling.

Required fields:

| Type | Required fields |
|---|---|
| `Canvas` | `canvas_id`, `space: overlay \| camera \| world`, `reference_resolution`, `scale_mode`, `sort_order` |
| `UiNode` | `node_id`, `parent`, `enabled`, `anchor_min`, `anchor_max`, `pivot`, `size_delta`, `local_position`, `z_order` |
| `Widget` | `widget_type`, `state`, `focusable`, `callbacks`, `serialized_properties` |
| `TextRun` | `font: AssetId`, `text`, `font_size`, `color`, `wrap`, `overflow`, `locale_hint` |

Rules:

- UI render extraction writes `UiBatch` entries into `RendererInput-v0`.
- UI input consumes platform input events and emits script/gameplay callbacks; it must not mutate platform internals directly.

## Audio-v0

Owner: Gate 16. Consumers: asset registry, ECS, scripting, gameplay framework, editor.

Required fields:

| Type | Required fields |
|---|---|
| `AudioAsset` | `asset_id`, `channels`, `sample_rate`, `duration_seconds`, `streaming`, `loop_points` |
| `AudioSource` | `clip: AssetId`, `volume`, `pitch`, `looping`, `spatial`, `min_distance`, `max_distance`, `group` |
| `AudioListener` | `entity`, `enabled`, `velocity_source` |
| `MixerGroup` | `group_id`, `parent`, `volume`, `mute`, `solo` |

Rules:

- Audio decode failures produce diagnostics and silence for that source, not a render or ECS failure.
- Audio thread callbacks must not perform blocking asset loads.

## ReleaseMetadata-v0

Owner: Gate 19. Consumers: packaging, QA, diagnostics, hot update, rollback.

```text
ReleaseMetadata {
  release_id: String,
  engine_version: EngineVersion,
  git_commit: Option<String>,
  build_profile: String,
  target_platform: PlatformProfile,
  asset_registry_hash: Hash,
  package_hashes: [Hash],
  symbols_artifact: Option<String>,
  qa_report_path: String,
  performance_report_path: String,
  signatures: [Signature]
}
```

Rules:

- A release artifact is invalid without metadata, hashes, QA report, and performance report.
- Diagnostic bundles must include enough version metadata to match logs/crashes to symbols and assets.

## CookedShader-v0

Owner: Gate 5 (cook) and Gate 2 (consumer). Producers: shader cook step under `engine-asset` (per `FD-005`). Consumers: every backend (`render-vulkan`, `render-opengl`, `render-dx12`).

A `CookedShader-v0` is the per-`(pipeline, variant_key, platform)` artifact emitted by the shader cook. Each artifact is wrapped by the standard `CookedAssetHeader` (`asset_kind = CookedShader`, payload via bincode) per `FD-006`. The contract is frozen by `FD-042`.

```text
CookedShader {
  contract_version: ContractVersion,            // "CookedShader-v0.1.0"
  pipeline_id: AssetId,                         // Pipeline authoring asset this artifact serves
  variant_key: u64,                             // bit-packed variant selector per FD-040
  target_platform: PlatformProfile,             // one of the enabled backend-* targets
  stages: ShaderStages,                         // per-stage compiled artifacts (vertex + fragment in v0)
  reflected_layout: PipelineLayoutInfo,         // descriptor sets + push constants per FD-041
  include_hashes: [IncludeRef],                 // recursive #include set per FD-038
  engine_defines: [{ name: String, value: String }],  // FD-038 injected macros at cook time
  cook_inputs_hash: Hash                        // sha256(source_hash || include_hashes || variant_key || engine_defines)
}

ShaderStages {
  vertex: ShaderStageBlob,                      // required in v0
  fragment: ShaderStageBlob,                    // required in v0
  compute: Option<ShaderStageBlob>              // reserved for OFQ-013; v0 always None
}

ShaderStageBlob {
  spirv: [u8],                                  // always present
  glsl: Option<String>,                         // present iff target_platform requires render-opengl (per FD-039)
  dxil: Option<[u8]>,                           // present iff target_platform requires render-dx12 (per FD-039)
  entry_point: String,                          // always "main" per FD-037
  source_hash: Hash                             // sha256 of LF-normalized source bytes
}

PipelineLayoutInfo {
  descriptor_sets: [DescriptorSetLayout],       // four entries for set=0..3 (set=3 may be empty per FD-041)
  push_constant_range: { offset: u32, size: u32, stage_flags: u32 },
  param_block_layout_hash: Hash                 // set=1, binding=0 UBO hash per FD-041
}

DescriptorSetLayout {
  set: u32,                                     // 0..3
  bindings: [DescriptorBinding]
}

DescriptorBinding {
  binding: u32,
  descriptor_type: UniformBuffer | StorageBuffer | SampledImage | Sampler | CombinedImageSampler,
  count: u32,                                   // 1 for non-arrayed bindings
  stage_flags: u32,                             // Vulkan VK_SHADER_STAGE_* bitmask
  name: String                                  // SPIR-V reflected name (e.g. "u_frame", "t_albedo")
}

IncludeRef {
  path: String,                                 // resolved path under assets/shaders/
  hash: Hash                                    // sha256 of LF-normalized included file
}
```

Rules:

- `stages.vertex` and `stages.fragment` are required in v0; `stages.compute` must be `None` until `OFQ-013` lands.
- `entry_point` is always `"main"`; the loader rejects with `SH0001 NonMainEntryPoint` otherwise.
- `target_platform` must match the consumer's runtime platform; the loader rejects with `SH0007 PlatformMismatch` otherwise.
- `glsl` is non-`None` only when `target_platform` enables `backend-opengl`; `dxil` only when `target_platform` enables `backend-dx12` (per `FD-039`).
- `reflected_layout.descriptor_sets` must contain exactly four entries (sets `0..=3`); missing sets are represented by an empty `bindings: []`. Backends use this layout shape directly when creating `vk::PipelineLayout` / `D3D12_ROOT_SIGNATURE` (per `FD-041`).
- `include_hashes` lists every file the SPIR-V depends on; Gate 6 hot-reload uses this to compute the reverse-dependency index from header file to consumer.
- `cook_inputs_hash` is the single equality check used by hot reload to decide whether a re-cook is needed; matching hash means the cooked artifact is still current.
- Cooked artifact `byte_size` recorded in `AssetRegistry-v0.CookedArtifact` covers header + payload as for all `-v0` artifacts.

PSO cache (per `FD-042`) is **not** part of this contract: it is per-machine binary cache, never shipped, never versioned, and may be deleted at any time.

## Cooked Asset Binary Format

Owner: cross-cutting (introduced by `FD-006`). Consumers: every cooked asset producer (Gate 5) and every cooked asset loader (renderer, scene loader, package installer, audio).

All cooked asset files share a fixed binary header followed by a bincode payload. The header is what loaders parse first; bincode never sees its bytes.

```text
CookedAssetHeader {                              // total 78 bytes
  magic: [u8; 8] = b"ENGCOOK\0",                 // file signature
  header_version: u16,                           // version of THIS header struct
  asset_kind: u16,                               // enum AssetKind (mesh, texture, scene, prefab, audio, navmesh, shader, pipeline, cooked_shader, ...)
  schema_version: { major: u16, minor: u16, patch: u16 },  // version of the cooked payload schema for this asset kind
  content_hash: [u8; 32],                        // sha256 of the bincode payload that follows (after compression)
  uncompressed_size: u64,                        // size of bincode payload BEFORE compression
  compressed_size: u64,                          // == uncompressed_size when compression == None
  compression: u8,                               // 0 = None, 1 = Zstd, 2 = Lz4 (reserved)
  reserved: [u8; 7]                              // must be zero in v0; readers ignore unknown bits
}
// followed by `compressed_size` bytes of bincode payload
```

Rules:

- Loader rejects the file with `CookedAsset::InvalidMagic` if `magic` does not equal `ENGCOOK\0`.
- Loader rejects with `CookedAsset::UnsupportedHeaderVersion` if `header_version` is unknown.
- Loader rejects with `CookedAsset::SchemaTooNew` if the payload `major` exceeds what the current binary understands; minor/patch are forward-compatible (loader uses serde defaults per `FD-009`).
- Loader verifies `content_hash` against the bytes that follow before invoking bincode; mismatch produces `CookedAsset::HashMismatch`.
- `compression == 1` (Zstd) is the recommended default for assets >= 4 KiB uncompressed; smaller assets use `compression == 0`.
- The bincode payload uses the default bincode 2.x configuration (little-endian, variable-length integer encoding) unless a per-kind override is documented in this section.
- Cooked artifact `byte_size` recorded in `AssetRegistry-v0.CookedArtifact` covers header + payload.
- All multi-byte header fields are little-endian.

Per-asset-kind payload schemas live next to their owning contract in this document (e.g. cooked mesh layout under a future Gate 5 mesh asset spec). When a payload schema changes:

- Additive optional fields with `#[serde(default)]` only require bumping `schema_version.minor`.
- Removing or renaming a payload field requires `schema_version.major` bump and a migration plan per `FD-009`.
- Asset kinds may not change their `asset_kind` enum value once shipped.
