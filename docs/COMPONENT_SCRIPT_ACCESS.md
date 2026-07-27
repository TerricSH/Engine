# Component script access audit

> GENERATED FILE — do not edit by hand. Regenerate with:
> `ENGINE_AUDIT_UPDATE=1 cargo test -p sandbox --locked --test component_script_access_audit`
> Source of truth: the component registry (`ComponentMeta::script_access` plus
> serde hooks) and the curated annotations in `engine-core/src/component_audit.rs`.

A component is reachable through the generic gameplay-script `Components` bridge
(`Components.Query` / `Components.Set`) only when its registry entry declares
`ScriptAccess::ReadOnly` or `ScriptAccess::ReadWrite` **and** provides both scene
serde hooks. `ReadOnly` answers queries but rejects writes with
`SCRIPT_COMPONENT_READ_ONLY`; `None` and `DedicatedApi` (Transform commands,
retained `UICanvas` handles) are rejected with `SCRIPT_COMPONENT_UNKNOWN`, exactly
like unregistered keys. Malformed payloads on writable components are rejected with
`SCRIPT_COMPONENT_PAYLOAD_INVALID`, and writes to unknown entities with
`SCRIPT_COMPONENT_TARGET_MISSING`.

| Type key | Script access | Serialize hook | Deserialize hook | Runtime reconciler | Caveat class | Decision notes |
|---|---|---|---|---|---|---|
| `engine.animation_player` | None | yes | yes | per frame (animation evaluation) | n/a — not script-accessible | None — runtime state-machine instance and playback caches are not scene-serializable; a generic write would silently drop them |
| `engine.audio_listener` | ReadWrite | yes | yes | per frame (audio output reconciler snapshots the enabled listener) | write takes effect live (on targets with audio output enabled) | ReadWrite — newly exposed; only field is `enabled`, pose comes from Transform |
| `engine.audio_source` | ReadWrite | yes | yes | per frame (audio output reconciler snapshots sources) | write takes effect live (on targets with audio output enabled) | ReadWrite — curated set |
| `engine.bounds` | None | no | no | per frame (frustum culling) | n/a — not script-accessible | None — no scene serde hooks registered; derived/render-side data |
| `engine.camera` | ReadWrite | yes | yes | per frame (render extraction builds views) | write takes effect live | ReadWrite — curated set; field writes are re-extracted next frame |
| `engine.canvas` | DedicatedApi | yes | yes | per frame (retained UI reconciliation) | dedicated API — retained UICanvas managed handles | DedicatedApi — scripts drive canvases through UICanvas handles, never the generic bridge |
| `engine.character_controller` | ReadOnly | yes | yes | per frame (character movement update reads parameters) | query only — writes rejected (SCRIPT_COMPONENT_READ_ONLY) | ReadOnly — query is safe, but the generic merge-write rebuilds the component from scene fields and would silently drop serde-skipped runtime state (pending commands, landing timer, ground normal) |
| `engine.gravity_source` | ReadWrite | yes | yes | per physics step (sources re-read from the ECS world each fixed step) | write takes effect live (next physics step) | ReadWrite — curated set |
| `engine.hlod_cluster` | ReadWrite | yes | yes | per frame (render extraction switches source/proxy cluster roles) | write takes effect live and can replace a complete source cluster | ReadWrite — cluster membership and proxy thresholds are validated and re-extracted next frame |
| `engine.ik_target` | None | yes | yes | per frame (IK solver consumes effectors) | n/a — not script-accessible | None — effector state is driven by animation/IK each frame; script-driven IK authoring is not a v1 surface |
| `engine.interactable` | ReadWrite | yes | yes | per physics query (interaction metadata extraction) | write takes effect live on the next interaction probe | ReadWrite — engine-owned targeting metadata; project scripts own the action's gameplay effect |
| `engine.light` | ReadWrite | yes | yes | per frame (render extraction builds light items) | write takes effect live | ReadWrite — curated set; field writes are re-extracted next frame |
| `engine.lod_group` | ReadWrite | yes | yes | per frame (render extraction selects assets by base-camera distance) | write takes effect live | ReadWrite — authored LOD/HLOD policy is validated and re-extracted next frame |
| `engine.name` | None | no | no | none (display metadata) | n/a — not script-accessible | None — no scene serde hooks registered; editor/display metadata only |
| `engine.nav_agent` | ReadWrite | yes | yes | per frame (navigation driver re-reads the agent) | write takes effect live; path following restarts (repath on next navigation update) | ReadWrite — newly exposed; serialized fields are plain configuration the driver re-reads each frame |
| `engine.physics.collider` | ReadWrite | yes | yes | load time (backend collider created when first seen) | write is scene-state only — does not re-sync an already-created physics collider | ReadWrite — curated set; caveat documented for game code |
| `engine.physics.destructible` | DedicatedApi | yes | yes | per damage command (health update and optional prefab fracture transaction) | dedicated API — Damage.Apply | DedicatedApi — the bounded typed API owns damage validation, one-shot break state, and fracture replacement |
| `engine.physics.joint` | DedicatedApi | yes | yes | per physics sync (persistent ids resolve to backend joint handles) | dedicated API — Physics.CreateJoint/UpdateJoint/RemoveJoint/Grab | DedicatedApi — the typed API validates cross-entity references and supports safe upsert/remove semantics |
| `engine.physics.physics_material` | ReadWrite | yes | yes | load time (read when the backend collider is created) | write is scene-state only — does not re-sync an already-created physics collider | ReadWrite — newly exposed; same caveat class as rigid_body/collider, which were already curated |
| `engine.physics.rigid_body` | ReadWrite | yes | yes | load time (backend body created when first seen) | write is scene-state only — does not re-sync an already-created physics body | ReadWrite — curated set; caveat documented for game code |
| `engine.prefab_instance_ref` | None | no | no | load time (prefab instantiation) | n/a — not script-accessible | None — internal prefab linkage, not authorable script data |
| `engine.ragdoll` | DedicatedApi | yes | yes | before/after physics step (body graph, ownership, and pose override) | dedicated API — Ragdoll.Activate/Recover/SnapToAnimation | DedicatedApi — typed transitions preserve animation/physics ownership and generated graph invariants |
| `engine.ragdoll_part` | None | yes | yes | per frame (internal generated-part cleanup) | n/a — not script-accessible | None — internal persistent ownership marker for generated ragdoll bodies and joints |
| `engine.renderable` | None | no | no | per frame (render extraction) | n/a — not script-accessible | None — no scene serde hooks registered; renderer-driven, not a v1 script surface |
| `engine.skeleton` | None | yes | yes | per frame (skinned extraction resolves cooked assets) | n/a — not script-accessible | None — structural skinning binding; scene-authored, not a v1 script surface |
| `engine.transform` | DedicatedApi | no | no | per frame (transform propagation + every consumer) | dedicated API — ScriptTransform commands | DedicatedApi — scripts use the dedicated, higher-fidelity Transform path |
| `engine.vfx.decal` | ReadWrite | yes | yes | per frame (lifetime update and render extraction) | write takes effect live; finite lifetime restarts | ReadWrite — plain surface configuration with intentionally transient elapsed lifetime |
| `engine.vfx.particle_emitter` | ReadWrite | yes | yes | per frame (CPU simulation and render extraction) | write takes effect live; transient particles and emitter clock restart | ReadWrite — authored configuration is safe to query/write; transient simulation state is intentionally not scene-serializable |
