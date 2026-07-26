//! Component script-access audit table generator.
//!
//! [`render_component_script_access_audit`] renders the checked-in
//! `docs/COMPONENT_SCRIPT_ACCESS.md` table from a component registry: the
//! type key, [`ScriptAccess`] level, and serde-hook presence come straight
//! from the registry entries, while the runtime-reconciler status, caveat
//! class, and per-component decision note come from the curated
//! [`AUDIT_ANNOTATIONS`] table below (reconciler behaviour cannot be derived
//! from the registry — it lives in the subsystem update loops).
//!
//! A registry entry without an annotation, or an annotation without a
//! registry entry, is a hard error: the audit must cover **every** registered
//! component, so adding a component forces an explicit access decision. The
//! drift guard lives in `crates/sandbox/tests/component_script_access_audit.rs`.

use engine_scene::registry::{ComponentRegistry, ScriptAccess};

/// Curated per-component audit metadata that the registry cannot express.
struct AuditAnnotation {
    /// Does a subsystem re-read the component each frame/step, or only when
    /// the scene loads?
    reconciler: &'static str,
    /// The caveat class of a generic-bridge write (or why none exists).
    caveat: &'static str,
    /// The access decision and its reason.
    decision: &'static str,
}

/// One annotation per component the canonical game runtime registers. Keep
/// the reconciler/caveat wording aligned with what the subsystem code
/// actually does; the drift guard fails when this table and the registry
/// disagree.
const AUDIT_ANNOTATIONS: &[(&str, AuditAnnotation)] = &[
    (
        "engine.name",
        AuditAnnotation {
            reconciler: "none (display metadata)",
            caveat: "n/a — not script-accessible",
            decision: "None — no scene serde hooks registered; editor/display metadata only",
        },
    ),
    (
        "engine.transform",
        AuditAnnotation {
            reconciler: "per frame (transform propagation + every consumer)",
            caveat: "dedicated API — ScriptTransform commands",
            decision: "DedicatedApi — scripts use the dedicated, higher-fidelity Transform path",
        },
    ),
    (
        "engine.renderable",
        AuditAnnotation {
            reconciler: "per frame (render extraction)",
            caveat: "n/a — not script-accessible",
            decision: "None — no scene serde hooks registered; renderer-driven, not a v1 script surface",
        },
    ),
    (
        "engine.camera",
        AuditAnnotation {
            reconciler: "per frame (render extraction builds views)",
            caveat: "write takes effect live",
            decision: "ReadWrite — curated set; field writes are re-extracted next frame",
        },
    ),
    (
        "engine.light",
        AuditAnnotation {
            reconciler: "per frame (render extraction builds light items)",
            caveat: "write takes effect live",
            decision: "ReadWrite — curated set; field writes are re-extracted next frame",
        },
    ),
    (
        "engine.interactable",
        AuditAnnotation {
            reconciler: "per physics query (interaction metadata extraction)",
            caveat: "write takes effect live on the next interaction probe",
            decision: "ReadWrite — engine-owned targeting metadata; project scripts own the action's gameplay effect",
        },
    ),
    (
        "engine.bounds",
        AuditAnnotation {
            reconciler: "per frame (frustum culling)",
            caveat: "n/a — not script-accessible",
            decision: "None — no scene serde hooks registered; derived/render-side data",
        },
    ),
    (
        "engine.prefab_instance_ref",
        AuditAnnotation {
            reconciler: "load time (prefab instantiation)",
            caveat: "n/a — not script-accessible",
            decision: "None — internal prefab linkage, not authorable script data",
        },
    ),
    (
        "engine.character_controller",
        AuditAnnotation {
            reconciler: "per frame (character movement update reads parameters)",
            caveat: "query only — writes rejected (SCRIPT_COMPONENT_READ_ONLY)",
            decision: "ReadOnly — query is safe, but the generic merge-write rebuilds the component from scene fields and would silently drop serde-skipped runtime state (pending commands, landing timer, ground normal)",
        },
    ),
    (
        "engine.physics.rigid_body",
        AuditAnnotation {
            reconciler: "load time (backend body created when first seen)",
            caveat: "write is scene-state only — does not re-sync an already-created physics body",
            decision: "ReadWrite — curated set; caveat documented for game code",
        },
    ),
    (
        "engine.physics.joint",
        AuditAnnotation {
            reconciler: "per physics sync (persistent ids resolve to backend joint handles)",
            caveat: "dedicated API — Physics.CreateJoint/UpdateJoint/RemoveJoint/Grab",
            decision: "DedicatedApi — the typed API validates cross-entity references and supports safe upsert/remove semantics",
        },
    ),
    (
        "engine.physics.destructible",
        AuditAnnotation {
            reconciler: "per damage command (health update and optional prefab fracture transaction)",
            caveat: "dedicated API — Damage.Apply",
            decision: "DedicatedApi — the bounded typed API owns damage validation, one-shot break state, and fracture replacement",
        },
    ),
    (
        "engine.physics.collider",
        AuditAnnotation {
            reconciler: "load time (backend collider created when first seen)",
            caveat: "write is scene-state only — does not re-sync an already-created physics collider",
            decision: "ReadWrite — curated set; caveat documented for game code",
        },
    ),
    (
        "engine.physics.physics_material",
        AuditAnnotation {
            reconciler: "load time (read when the backend collider is created)",
            caveat: "write is scene-state only — does not re-sync an already-created physics collider",
            decision: "ReadWrite — newly exposed; same caveat class as rigid_body/collider, which were already curated",
        },
    ),
    (
        "engine.gravity_source",
        AuditAnnotation {
            reconciler: "per physics step (sources re-read from the ECS world each fixed step)",
            caveat: "write takes effect live (next physics step)",
            decision: "ReadWrite — curated set",
        },
    ),
    (
        "engine.canvas",
        AuditAnnotation {
            reconciler: "per frame (retained UI reconciliation)",
            caveat: "dedicated API — retained UICanvas managed handles",
            decision: "DedicatedApi — scripts drive canvases through UICanvas handles, never the generic bridge",
        },
    ),
    (
        "engine.audio_source",
        AuditAnnotation {
            reconciler: "per frame (audio output reconciler snapshots sources)",
            caveat: "write takes effect live (on targets with audio output enabled)",
            decision: "ReadWrite — curated set",
        },
    ),
    (
        "engine.audio_listener",
        AuditAnnotation {
            reconciler: "per frame (audio output reconciler snapshots the enabled listener)",
            caveat: "write takes effect live (on targets with audio output enabled)",
            decision: "ReadWrite — newly exposed; only field is `enabled`, pose comes from Transform",
        },
    ),
    (
        "engine.animation_player",
        AuditAnnotation {
            reconciler: "per frame (animation evaluation)",
            caveat: "n/a — not script-accessible",
            decision: "None — runtime state-machine instance and playback caches are not scene-serializable; a generic write would silently drop them",
        },
    ),
    (
        "engine.ragdoll",
        AuditAnnotation {
            reconciler: "before/after physics step (body graph, ownership, and pose override)",
            caveat: "dedicated API — Ragdoll.Activate/Recover/SnapToAnimation",
            decision: "DedicatedApi — typed transitions preserve animation/physics ownership and generated graph invariants",
        },
    ),
    (
        "engine.ragdoll_part",
        AuditAnnotation {
            reconciler: "per frame (internal generated-part cleanup)",
            caveat: "n/a — not script-accessible",
            decision: "None — internal persistent ownership marker for generated ragdoll bodies and joints",
        },
    ),
    (
        "engine.skeleton",
        AuditAnnotation {
            reconciler: "per frame (skinned extraction resolves cooked assets)",
            caveat: "n/a — not script-accessible",
            decision: "None — structural skinning binding; scene-authored, not a v1 script surface",
        },
    ),
    (
        "engine.ik_target",
        AuditAnnotation {
            reconciler: "per frame (IK solver consumes effectors)",
            caveat: "n/a — not script-accessible",
            decision: "None — effector state is driven by animation/IK each frame; script-driven IK authoring is not a v1 surface",
        },
    ),
    (
        "engine.nav_agent",
        AuditAnnotation {
            reconciler: "per frame (navigation driver re-reads the agent)",
            caveat: "write takes effect live; path following restarts (repath on next navigation update)",
            decision: "ReadWrite — newly exposed; serialized fields are plain configuration the driver re-reads each frame",
        },
    ),
    (
        "engine.vfx.particle_emitter",
        AuditAnnotation {
            reconciler: "per frame (CPU simulation and render extraction)",
            caveat: "write takes effect live; transient particles and emitter clock restart",
            decision: "ReadWrite — authored configuration is safe to query/write; transient simulation state is intentionally not scene-serializable",
        },
    ),
    (
        "engine.vfx.decal",
        AuditAnnotation {
            reconciler: "per frame (lifetime update and render extraction)",
            caveat: "write takes effect live; finite lifetime restarts",
            decision: "ReadWrite — plain surface configuration with intentionally transient elapsed lifetime",
        },
    ),
];

fn access_label(access: ScriptAccess) -> &'static str {
    match access {
        ScriptAccess::None => "None",
        ScriptAccess::ReadOnly => "ReadOnly",
        ScriptAccess::ReadWrite => "ReadWrite",
        ScriptAccess::DedicatedApi => "DedicatedApi",
    }
}

fn hook_label(present: bool) -> &'static str {
    if present {
        "yes"
    } else {
        "no"
    }
}

/// Render the audit document for every entry in `registry`.
///
/// Returns `Err` with human-readable problems when the registry and the
/// curated annotation table disagree in either direction.
pub fn render_component_script_access_audit(
    registry: &ComponentRegistry,
) -> Result<String, String> {
    let mut entries: Vec<_> = registry.iter().collect();
    entries.sort_by_key(|extension| extension.meta.type_id);

    let mut problems = Vec::new();
    for extension in &entries {
        if !AUDIT_ANNOTATIONS
            .iter()
            .any(|(type_id, _)| *type_id == extension.meta.type_id)
        {
            problems.push(format!(
                "registered component '{}' has no audit annotation; add one to AUDIT_ANNOTATIONS in engine-core/src/component_audit.rs",
                extension.meta.type_id
            ));
        }
    }
    for (type_id, _) in AUDIT_ANNOTATIONS {
        if !entries
            .iter()
            .any(|extension| extension.meta.type_id == *type_id)
        {
            problems.push(format!(
                "audit annotation for '{type_id}' has no matching registry entry; remove or update the stale annotation in engine-core/src/component_audit.rs"
            ));
        }
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }

    let mut document = String::new();
    document.push_str(
        "# Component script access audit\n\
         \n\
         > GENERATED FILE — do not edit by hand. Regenerate with:\n\
         > `ENGINE_AUDIT_UPDATE=1 cargo test -p sandbox --locked --test component_script_access_audit`\n\
         > Source of truth: the component registry (`ComponentMeta::script_access` plus\n\
         > serde hooks) and the curated annotations in `engine-core/src/component_audit.rs`.\n\
         \n\
         A component is reachable through the generic gameplay-script `Components` bridge\n\
         (`Components.Query` / `Components.Set`) only when its registry entry declares\n\
         `ScriptAccess::ReadOnly` or `ScriptAccess::ReadWrite` **and** provides both scene\n\
         serde hooks. `ReadOnly` answers queries but rejects writes with\n\
         `SCRIPT_COMPONENT_READ_ONLY`; `None` and `DedicatedApi` (Transform commands,\n\
         retained `UICanvas` handles) are rejected with `SCRIPT_COMPONENT_UNKNOWN`, exactly\n\
         like unregistered keys. Malformed payloads on writable components are rejected with\n\
         `SCRIPT_COMPONENT_PAYLOAD_INVALID`, and writes to unknown entities with\n\
         `SCRIPT_COMPONENT_TARGET_MISSING`.\n\
         \n\
         | Type key | Script access | Serialize hook | Deserialize hook | Runtime reconciler | Caveat class | Decision notes |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for extension in entries {
        let meta = &extension.meta;
        let (_, annotation) = AUDIT_ANNOTATIONS
            .iter()
            .find(|(type_id, _)| *type_id == meta.type_id)
            .expect("annotation presence checked above");
        document.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} | {} |\n",
            meta.type_id,
            access_label(meta.script_access),
            hook_label(extension.serialize.is_some()),
            hook_label(extension.deserialize.is_some()),
            annotation.reconciler,
            annotation.caveat,
            annotation.decision,
        ));
    }
    Ok(document)
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_scene::{
        Component, ComponentExtension, ComponentMeta, ComponentStorageDyn, SparseSet,
    };

    struct AuditDummy;

    impl Component for AuditDummy {
        const TYPE_ID: &'static str = "test.audit_dummy";
    }

    fn register_dummy(registry: &mut ComponentRegistry, type_id: &'static str) {
        // Leak-free for tests: register a component whose meta type_id is the
        // provided string by using the dummy storage with an overridden meta.
        registry
            .register(ComponentExtension {
                meta: ComponentMeta {
                    type_id,
                    display_name: "Audit Dummy",
                    schema_version: (0, 1, 0),
                    has_editor: false,
                    script_access: ScriptAccess::None,
                },
                storage_factory: || -> Box<dyn ComponentStorageDyn> {
                    Box::new(SparseSet::<AuditDummy>::new())
                },
                serialize: None,
                deserialize: None,
            })
            .expect("register audit dummy");
    }

    fn canonical_core_registry() -> ComponentRegistry {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        registry
    }

    #[test]
    fn unannotated_registry_entries_are_a_hard_error() {
        let mut registry = canonical_core_registry();
        register_dummy(&mut registry, "test.unannotated");
        let error = render_component_script_access_audit(&registry)
            .expect_err("unannotated component must fail the audit");
        assert!(error.contains("test.unannotated"), "{error}");
    }

    #[test]
    fn stale_annotations_are_a_hard_error() {
        // The core registry alone never carries subsystem components, so
        // their annotations are stale relative to it.
        let registry = canonical_core_registry();
        let error = render_component_script_access_audit(&registry)
            .expect_err("annotations without registry entries must fail the audit");
        assert!(error.contains("engine.audio_source"), "{error}");
    }

    #[test]
    fn table_renders_registry_derived_columns() {
        let mut registry = ComponentRegistry::new();
        registry.register_core();
        // Add the remaining canonical entries with minimal stubs so the
        // annotation table is fully covered.
        for (type_id, access) in [
            ("engine.character_controller", ScriptAccess::ReadOnly),
            ("engine.physics.rigid_body", ScriptAccess::ReadWrite),
            ("engine.physics.joint", ScriptAccess::DedicatedApi),
            ("engine.physics.destructible", ScriptAccess::DedicatedApi),
            ("engine.physics.collider", ScriptAccess::ReadWrite),
            ("engine.physics.physics_material", ScriptAccess::ReadWrite),
            ("engine.gravity_source", ScriptAccess::ReadWrite),
            ("engine.canvas", ScriptAccess::DedicatedApi),
            ("engine.audio_source", ScriptAccess::ReadWrite),
            ("engine.audio_listener", ScriptAccess::ReadWrite),
            ("engine.animation_player", ScriptAccess::None),
            ("engine.ragdoll", ScriptAccess::DedicatedApi),
            ("engine.ragdoll_part", ScriptAccess::None),
            ("engine.skeleton", ScriptAccess::None),
            ("engine.ik_target", ScriptAccess::None),
            ("engine.nav_agent", ScriptAccess::ReadWrite),
            ("engine.vfx.particle_emitter", ScriptAccess::ReadWrite),
            ("engine.vfx.decal", ScriptAccess::ReadWrite),
        ] {
            registry
                .register(ComponentExtension {
                    meta: ComponentMeta {
                        type_id,
                        display_name: "Stub",
                        schema_version: (0, 1, 0),
                        has_editor: false,
                        script_access: access,
                    },
                    storage_factory: || -> Box<dyn ComponentStorageDyn> {
                        Box::new(SparseSet::<AuditDummy>::new())
                    },
                    serialize: None,
                    deserialize: None,
                })
                .expect("register stub");
        }

        let document = render_component_script_access_audit(&registry)
            .expect("fully annotated registry renders");
        assert!(document.contains("| `engine.camera` | ReadWrite | yes | yes |"));
        assert!(document.contains("| `engine.transform` | DedicatedApi | no | no |"));
        assert!(document.contains("| `engine.character_controller` | ReadOnly | no | no |"));
        assert!(document.contains("| `engine.nav_agent` | ReadWrite | no | no |"));
        assert!(document.contains("SCRIPT_COMPONENT_READ_ONLY"));
        // Rows are sorted by type key.
        let camera = document.find("`engine.audio_listener`").unwrap();
        let transform = document.find("`engine.transform`").unwrap();
        assert!(camera < transform);
    }
}
