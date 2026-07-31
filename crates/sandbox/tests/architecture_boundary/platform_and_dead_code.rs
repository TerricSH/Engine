#[test]
fn platform_and_backend_dependencies_stay_at_adapter_boundaries() {
    let root = workspace_root();
    let core_manifest = fs::read_to_string(root.join("crates/engine-core/Cargo.toml"))
        .expect("engine-core manifest");
    for forbidden in [
        "raw-window-handle",
        "render-vulkan",
        "render-dx12",
        "winit",
        "tao",
    ] {
        assert!(
            !core_manifest.contains(forbidden),
            "engine-core must remain backend-neutral; found dependency '{forbidden}'"
        );
    }

    let crates = root.join("crates");
    let allow_raw_handles = ["engine-editor-host", "platform", "render-vulkan"];
    let allow_winit = ["platform"];
    let allow_tao = ["engine-editor-host"];
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let crate_dir = entry.expect("crate directory entry").path();
        let Some(crate_name) = crate_dir.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let manifest = crate_dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest).expect("crate manifest");
        for (dependency, allowlist) in [
            ("raw-window-handle", allow_raw_handles.as_slice()),
            ("winit.workspace", allow_winit.as_slice()),
            ("tao.workspace", allow_tao.as_slice()),
        ] {
            if text.contains(dependency) {
                assert!(
                    allowlist.contains(&crate_name),
                    "{crate_name} bypasses the platform/backend adapter with '{dependency}'"
                );
            }
        }
    }

    let vulkan =
        fs::read_to_string(root.join("crates/render-vulkan/src/lib.rs")).expect("Vulkan facade");
    assert!(
        vulkan.contains("pub fn create_backend_renderer_for_surface"),
        "the platform-surface Vulkan adapter belongs to render-vulkan"
    );

    let platform =
        fs::read_to_string(root.join("crates/platform/src/lib.rs")).expect("platform facade");
    assert!(
        !platform.contains("pub use winit"),
        "platform must not re-export its windowing implementation"
    );
    for public_leak in [
        "pub fn on_create(&mut self, window: Arc<Window>)",
        "pub fn on_event(&mut self, window: &Window",
        "pub fn from_winit",
    ] {
        assert!(
            !platform.contains(public_leak),
            "platform public API leaks a winit type through '{public_leak}'"
        );
    }

    let sandbox_manifest =
        fs::read_to_string(root.join("crates/sandbox/Cargo.toml")).expect("sandbox manifest");
    assert!(
        !sandbox_manifest.contains("raw-window-handle")
            && !sandbox_manifest.contains("winit.workspace"),
        "sandbox must consume opaque platform windows and surfaces"
    );
    let mut sandbox_sources = Vec::new();
    visit_rust_sources(&root.join("crates/sandbox/src"), &mut sandbox_sources);
    for source in sandbox_sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        assert!(
            !text.contains("platform::winit")
                && !text.contains("raw_window_handle")
                && !text.contains("winit::"),
            "{} bypasses the opaque platform window/surface contract",
            source.display()
        );
    }
}

#[test]
fn project_app_uses_real_modules_instead_of_textual_production_includes() {
    let facade = fs::read_to_string(workspace_root().join("crates/sandbox/src/project_app.rs"))
        .expect("project app facade");
    for module in ["assets", "headless", "run", "transitions", "windowed"] {
        assert!(
            facade.contains(&format!("mod {module};")),
            "project_app must declare its {module} implementation as a Rust module"
        );
        assert!(
            !facade.contains(&format!("include!(\"project_app/{module}.rs\")")),
            "project_app must not textually include production module {module}"
        );
    }
}

#[test]
fn sandbox_editor_domains_use_real_production_modules() {
    let root = workspace_root();
    for (facade, include_prefix, modules) in [
        (
            "crates/sandbox/src/editor_asset_ops.rs",
            "editor_asset_ops",
            &[
                "assets",
                "catalog",
                "deletion",
                "folders",
                "identity",
                "paths",
                "transaction",
            ][..],
        ),
        (
            "crates/sandbox/src/editor_build_ops.rs",
            "editor_build_ops",
            &["completion", "process_io", "service", "task", "validation"][..],
        ),
        (
            "crates/sandbox/src/editor_app/snapshot.rs",
            "snapshot",
            &["helpers", "overview", "panels"][..],
        ),
        (
            "crates/sandbox/src/editor_app/protocol.rs",
            "protocol",
            &["assets_project", "core", "runtime_viewport", "scene_entity"][..],
        ),
    ] {
        let facade_path = root.join(facade);
        let source = fs::read_to_string(&facade_path)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", facade_path.display()));
        for module in modules {
            assert!(
                source.lines().any(|line| line.trim() == format!("mod {module};")),
                "{facade} must declare production child module '{module}' with `mod`"
            );
            assert!(
                !source.contains(&format!("include!(\"{include_prefix}/{module}.rs\")")),
                "{facade} must not flatten production module '{module}' with include!"
            );
        }
    }
}

#[test]
fn commented_out_rust_implementations_cannot_accumulate() {
    let crates = workspace_root().join("crates");
    let mut sources = Vec::new();
    for entry in fs::read_dir(crates).expect("workspace crates directory") {
        let source_dir = entry.expect("crate directory entry").path().join("src");
        if source_dir.is_dir() {
            visit_rust_sources(&source_dir, &mut sources);
        }
    }

    let code_markers = [
        "fn ",
        "async fn ",
        "struct ",
        "enum ",
        "trait ",
        "impl ",
        "let ",
    ];
    for source in sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        for (line_number, line) in text.lines().enumerate() {
            let trimmed = line.trim_start();
            let Some(comment) = trimmed.strip_prefix("//") else {
                continue;
            };
            if comment.starts_with('/') || comment.starts_with('!') {
                continue;
            }
            let comment = comment.trim_start();
            let looks_like_item = code_markers
                .iter()
                .any(|marker| comment.starts_with(marker))
                || (comment.starts_with("pub ")
                    && code_markers
                        .iter()
                        .any(|marker| comment[4..].starts_with(marker)));
            assert!(
                !looks_like_item,
                "{}:{} contains a commented-out implementation; delete it or restore it as code",
                source.display(),
                line_number + 1
            );
        }
    }
}

#[test]
fn removed_custom_editor_ui_cannot_reenter_production_sources() {
    let root = workspace_root();
    for retired in [
        "crates/engine-editor/src/editor_core.rs",
        "crates/engine-editor/src/editor_ui.rs",
        "crates/engine-editor/src/build.rs",
        "crates/engine-editor/src/debug_views.rs",
        "crates/engine-editor/src/hierarchy.rs",
        "crates/engine-editor/src/inspector.rs",
        "crates/engine-editor/src/script_build.rs",
        "crates/engine-editor/src/script_inspector.rs",
        "crates/engine-editor/src/plugin.rs",
        "crates/engine-editor/src/scene_view.rs",
    ] {
        assert!(
            !root.join(retired).exists(),
            "the retired custom editor implementation must stay deleted: {retired}"
        );
    }

    let mut sources = Vec::new();
    visit_rust_sources(&root.join("crates/engine-editor/src"), &mut sources);
    visit_rust_sources(&root.join("crates/sandbox/src"), &mut sources);
    let forbidden = [
        "EditorUi",
        "editor_ui::",
        "EditorCore",
        "EditorDisabled",
        "ScriptBuildManager",
        "build_csharp_project",
        "draw_gizmo(",
        "LegacyInspectorPanel",
        "HierarchyPanel",
        "InspectorContext",
        "ScriptInspector",
        "UiKey",
        "UiInteractionPhase",
        "UiInteractionStamp",
        "SequencedCommand",
        "SequencedSceneViewAction",
        "draw_asset_browser",
        "draw_material_editor",
        "set_diagnostics(",
        "add_diagnostics(",
        "EditorPluginRegistry",
        "MaterialPreviewRequest",
        "render_material_preview_rgba8",
        "orbit_projection_matrix",
    ];
    for source in sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        for symbol in forbidden {
            assert!(
                !text.contains(symbol),
                "retired editor UI symbol '{symbol}' re-entered {}",
                source.display()
            );
        }
    }
}
