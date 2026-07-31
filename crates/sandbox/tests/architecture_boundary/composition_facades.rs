#[test]
fn engine_core_composition_root_stays_bounded_and_uses_leaf_features() {
    let root = workspace_root();
    let source_root = root.join("crates/engine-core/src");
    let facade = source_root.join("lib.rs");
    assert_facade_budget(&facade, 128, 32);
    assert_facade_budget(&source_root.join("game_loop.rs"), 400, 64);
    assert_facade_budget(&source_root.join("cell_stream.rs"), 160, 8);
    assert_facade_budget(&source_root.join("cooked_assets.rs"), 100, 8);
    assert_facade_budget(&source_root.join("runtime/scripting.rs"), 300, 32);
    assert_facade_budget(&source_root.join("runtime/state.rs"), 450, 20);

    for (facade, production_modules) in [
        (
            "cell_stream.rs",
            &["config", "driver", "state", "validation", "world_positions"][..],
        ),
        (
            "cooked_assets.rs",
            &["decode", "decoded", "runtime", "types", "validation"][..],
        ),
    ] {
        let facade_path = source_root.join(facade);
        let facade_source = fs::read_to_string(&facade_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", facade_path.display()));
        assert!(
            !facade_source.contains("include!("),
            "{} must compose production code through Rust modules, not textual include expansion",
            facade_path.display()
        );

        let module_root = facade_path.with_extension("");
        for module in production_modules {
            assert!(
                facade_source
                    .lines()
                    .any(|line| line.trim() == format!("mod {module};")),
                "{} must declare production child module '{module}' with `mod`",
                facade_path.display()
            );
            let module_path = module_root.join(format!("{module}.rs"));
            let module_source = fs::read_to_string(&module_path).unwrap_or_else(|error| {
                panic!("could not read {}: {error}", module_path.display())
            });
            assert!(
                !module_source.contains("include!("),
                "{} must remain a real production module, not an include fragment",
                module_path.display()
            );
        }
    }

    for required_boundary in [
        "runtime/mod.rs",
        "runtime/assets.rs",
        "runtime/builder.rs",
        "runtime/rendering.rs",
        "runtime/scripting.rs",
        "runtime/scripting/commands.rs",
        "runtime/scripting/context.rs",
        "runtime/scripting/extended_commands.rs",
        "runtime/scripting/lifecycle.rs",
        "runtime/scripting/queries.rs",
        "runtime/scripting/state.rs",
        "runtime/scripting/world.rs",
        "runtime/state.rs",
        "game_loop/animation.rs",
        "game_loop/audio.rs",
        "game_loop/character.rs",
        "game_loop/frame.rs",
        "game_loop/navigation.rs",
        "game_loop/physics.rs",
        "game_loop/save.rs",
        "game_loop/script_input.rs",
        "game_loop/tests.rs",
        "game_loop/ui.rs",
        "game_loop/world_origin.rs",
        "script_commands/mod.rs",
    ] {
        assert!(
            source_root.join(required_boundary).is_file(),
            "engine-core boundary module is missing: {required_boundary}"
        );
    }

    let runtime_state =
        fs::read_to_string(source_root.join("runtime/state.rs")).expect("runtime state source");
    assert!(
        runtime_state.contains("pub(crate) scripting: super::scripting::ScriptRuntimeState"),
        "managed scripting queues must remain grouped behind one leaf-feature state object"
    );
    for retired_field in [
        "pub(crate) script_engine:",
        "pub(crate) script_host_name:",
        "pub(crate) script_pointer:",
        "pub(crate) pending_physics_queries:",
        "pub(crate) pending_component_queries:",
    ] {
        assert!(
            !runtime_state.contains(retired_field),
            "EngineRuntime reintroduced conditional scripting field '{retired_field}'"
        );
    }

    for module in ["cell_stream", "cooked_assets", "runtime/scripting"] {
        let module_dir = source_root.join(module);
        let mut fragments = Vec::new();
        visit_rust_sources(&module_dir, &mut fragments);
        for fragment in fragments {
            if fragment.file_name().and_then(|name| name.to_str()) == Some("tests.rs") {
                continue;
            }
            assert!(
                source_line_count(&fragment) <= 1_000,
                "{} grew beyond the engine-core adapter-fragment budget",
                fragment.display()
            );
        }
    }

    let mut sources = Vec::new();
    visit_rust_sources(&source_root, &mut sources);
    for source in sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        for compatibility_alias in ["runtime-subsystems", "gameplay", "terrain"] {
            let forbidden = format!("feature = \"{compatibility_alias}\"");
            assert!(
                !text.contains(&forbidden),
                "{} gates implementation with compatibility feature '{compatibility_alias}'; \
                 use the owning leaf feature instead",
                source.display()
            );
        }
    }
}

#[test]
fn sandbox_composition_facades_stay_bounded() {
    let source_root = workspace_root().join("crates/sandbox/src");
    for (facade, max_lines, max_cfg_sites) in [
        ("editor_app.rs", 600, 32),
        ("project_cli.rs", 250, 24),
        ("project_scripts.rs", 300, 24),
    ] {
        assert_facade_budget(&source_root.join(facade), max_lines, max_cfg_sites);
        assert!(
            source_root.join(facade).with_extension("").is_dir(),
            "{facade} must remain a facade over owned child modules"
        );
    }
}

#[test]
fn renderer_facades_and_backend_fragments_stay_bounded() {
    let root = workspace_root();
    for (facade, modules) in [
        (
            "crates/render-vulkan/src/scene_renderer.rs",
            &[
                "backend",
                "drop",
                "forward",
                "frame",
                "lifecycle",
                "post_process",
                "resources",
                "shadow",
                "state",
                "support",
                "timing",
            ][..],
        ),
        (
            "crates/render-dx12/src/scene_renderer.rs",
            &[
                "backend",
                "fallback",
                "forward",
                "lifecycle",
                "pipelines",
                "post_process",
                "resources",
                "shadow",
                "support",
            ][..],
        ),
    ] {
        let facade_path = root.join(facade);
        assert_facade_budget(&facade_path, 150, 24);
        let facade_source = fs::read_to_string(&facade_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", facade_path.display()));
        assert!(
            !facade_source.contains("include!("),
            "{facade} must declare real Rust submodules; production include! fragments erase module boundaries"
        );
        for module in modules {
            assert!(
                facade_source
                    .lines()
                    .any(|line| line.trim() == format!("mod {module};")),
                "{facade} must declare backend child module '{module}' with `mod`"
            );
        }

        let module_dir = facade_path.with_extension("");
        assert!(
            module_dir.is_dir(),
            "{facade} must remain a facade over backend-owned fragments"
        );
        let mut fragments = Vec::new();
        visit_rust_sources(&module_dir, &mut fragments);
        for fragment in fragments {
            assert!(
                source_line_count(&fragment) <= 1_000,
                "{} grew beyond the backend-fragment budget; split by pass or resource domain",
                fragment.display()
            );
        }
    }

    let shared = root.join("crates/engine-renderer/src/backend_shared.rs");
    assert_facade_budget(&shared, 300, 8);
    for required in ["environment.rs", "frame.rs", "post_process.rs", "ui.rs"] {
        assert!(
            shared.with_extension("").join(required).is_file(),
            "backend-neutral render planning module is missing: {required}"
        );
    }
}

#[test]
fn renderer_backend_implementation_facades_use_real_modules() {
    let root = workspace_root();
    for (facade, modules) in [
        (
            "crates/render-dx12/src/device.rs",
            &[
                "platform",
                "trait_frame",
                "trait_pipeline",
                "trait_resources",
            ][..],
        ),
        (
            "crates/render-opengl/src/device.rs",
            &["frame", "framebuffers", "pipelines", "resources"][..],
        ),
        (
            "crates/render-opengl/src/encoder.rs",
            &["bindings", "constants", "draw", "pass"][..],
        ),
        (
            "crates/render-vulkan/src/device_impl/device_trait.rs",
            &["frame", "pipeline", "render_targets", "resources"][..],
        ),
        (
            "crates/render-vulkan/src/device_impl/hdr.rs",
            &["cleanup", "forward", "targets", "tone_mapping"][..],
        ),
        ("crates/render-vulkan/src/device_impl/mod.rs", &["base"][..]),
        (
            "crates/render-vulkan/src/device_impl/base/mod.rs",
            &["construction", "pipeline_helpers", "runtime"][..],
        ),
        (
            "crates/render-vulkan/src/device_impl/shadow.rs",
            &["cascade_math", "resources"][..],
        ),
        (
            "crates/render-vulkan/src/scene_renderer/forward.rs",
            &["main", "particles"][..],
        ),
    ] {
        let path = root.join(facade);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()));
        let production = source
            .split("#[cfg(test)]")
            .next()
            .expect("production source prefix");
        assert!(
            !production.contains("include!("),
            "{facade} must not flatten production implementation through include!"
        );
        for module in modules {
            assert!(
                production
                    .lines()
                    .any(|line| line.trim() == format!("mod {module};")),
                "{facade} must declare production child module '{module}' with `mod`"
            );
        }
    }

    let forward_main =
        fs::read_to_string(root.join("crates/render-vulkan/src/scene_renderer/forward/main.rs"))
            .expect("Vulkan forward main source");
    assert!(
        !forward_main.contains("include!("),
        "Vulkan forward main must call the particle submodule instead of including a statement fragment"
    );
}

#[test]
fn cross_crate_contracts_have_one_definition_site() {
    let root = workspace_root();
    let crates = root.join("crates");
    let mut sources = Vec::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let source_dir = entry.expect("crate directory entry").path().join("src");
        if source_dir.is_dir() {
            visit_rust_sources(&source_dir, &mut sources);
        }
    }

    for (declaration, owner) in [
        ("pub enum ShaderStage {", "crates/render-core/src/types.rs"),
        ("pub enum IndexFormat {", "crates/render-core/src/types.rs"),
        (
            "pub enum TextureFormat {",
            "crates/render-core/src/types.rs",
        ),
        (
            "pub struct VertexLayout {",
            "crates/render-core/src/types.rs",
        ),
        (
            "pub struct VertexAttribute {",
            "crates/render-core/src/types.rs",
        ),
        (
            "pub enum LightKind {",
            "crates/engine-serialize/src/lighting.rs",
        ),
        (
            "pub struct LogicAsset {",
            "crates/engine-serialize/src/logic.rs",
        ),
    ] {
        let owners = sources
            .iter()
            .filter(|source| {
                fs::read_to_string(source)
                    .expect("Rust source")
                    .lines()
                    .any(|line| line.trim_start().starts_with(declaration))
            })
            .collect::<Vec<_>>();
        assert_eq!(
            owners.len(),
            1,
            "'{declaration}' must have exactly one definition site; found {owners:?}"
        );
        assert_eq!(
            owners[0],
            &root.join(owner),
            "'{declaration}' moved away from its canonical owner"
        );
    }

    assert!(
        !root
            .join("crates/engine-renderer/src/shader_compiler.rs")
            .exists(),
        "the unreferenced duplicate shader compiler must stay deleted"
    );
}
