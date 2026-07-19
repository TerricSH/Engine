//! Drift guard for `docs/COMPONENT_SCRIPT_ACCESS.md`.
//!
//! Assembles the canonical game-runtime component registry (the same set
//! `EngineRuntimeBuilder` installs for a full game build: core + character +
//! physics + UI + audio + animation + navigation) and verifies that the
//! checked-in audit table matches what the registry actually declares.
//!
//! When the registry or the curated annotations change on purpose, refresh
//! the document with:
//!
//! ```text
//! ENGINE_AUDIT_UPDATE=1 cargo test -p sandbox --locked --test component_script_access_audit
//! ```

use std::path::PathBuf;

use engine_scene::registry::ComponentRegistry;

/// The canonical registry a fully-featured game runtime exposes to scripts.
fn canonical_game_registry() -> ComponentRegistry {
    let mut components = ComponentRegistry::new();
    components.register_core();
    engine_character::register_character_extensions(&mut components, None);
    engine_physics::register_physics_extensions(&mut components, None);
    engine_ui::register_ui_extensions(&mut components);

    let mut assets = engine_scene::registry::AssetTypeRegistry::new();
    engine_audio::register_audio_extensions(&mut components, &mut assets);
    engine_nav::register_nav_extensions(&mut components, None, &mut assets);

    let mut render_extensions = engine_renderer::RenderExtensionRegistry::new();
    let mut debug_draw = engine_renderer::DebugDrawRegistry::new();
    engine_animation::register_animation_extensions(
        &mut components,
        &mut assets,
        &mut render_extensions,
        &mut debug_draw,
    );
    components
}

fn audit_document_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("docs/COMPONENT_SCRIPT_ACCESS.md")
}

#[test]
fn component_script_access_audit_table_is_up_to_date() {
    let registry = canonical_game_registry();
    let rendered = engine_core::component_audit::render_component_script_access_audit(&registry)
        .unwrap_or_else(|problems| panic!("audit table cannot be rendered:\n{problems}"));

    let path = audit_document_path();
    if std::env::var_os("ENGINE_AUDIT_UPDATE").is_some() {
        std::fs::write(&path, &rendered)
            .unwrap_or_else(|error| panic!("failed to write {}: {error}", path.display()));
        return;
    }

    let checked_in = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "{} is missing ({error}); generate it with `ENGINE_AUDIT_UPDATE=1 cargo test -p sandbox --locked --test component_script_access_audit`",
            path.display()
        )
    });
    // Normalize line endings so the guard is stable across checkouts.
    let checked_in = checked_in.replace("\r\n", "\n");
    assert_eq!(
        rendered, checked_in,
        "docs/COMPONENT_SCRIPT_ACCESS.md drifted from the component registry; regenerate it with `ENGINE_AUDIT_UPDATE=1 cargo test -p sandbox --locked --test component_script_access_audit`"
    );
}

#[test]
fn curated_bridge_components_keep_their_access_levels() {
    use engine_scene::ScriptAccess;

    let registry = canonical_game_registry();
    let access = |type_id: &str| {
        registry
            .get(type_id)
            .unwrap_or_else(|| panic!("{type_id} is not registered"))
            .meta
            .script_access
    };

    // The previously curated set must not regress.
    for type_id in [
        "engine.camera",
        "engine.light",
        "engine.audio_source",
        "engine.physics.rigid_body",
        "engine.physics.collider",
        "engine.gravity_source",
    ] {
        assert_eq!(
            access(type_id),
            ScriptAccess::ReadWrite,
            "{type_id} regressed from the curated ReadWrite set"
        );
    }

    // Newly exposed components (see docs/COMPONENT_SCRIPT_ACCESS.md).
    for type_id in [
        "engine.audio_listener",
        "engine.physics.physics_material",
        "engine.nav_agent",
    ] {
        assert_eq!(access(type_id), ScriptAccess::ReadWrite, "{type_id}");
    }
    assert_eq!(
        access("engine.character_controller"),
        ScriptAccess::ReadOnly
    );
    assert_eq!(access("engine.canvas"), ScriptAccess::DedicatedApi);
    assert_eq!(access("engine.transform"), ScriptAccess::DedicatedApi);
}
