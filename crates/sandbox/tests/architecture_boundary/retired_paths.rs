#[test]
fn retired_render_and_asset_compatibility_paths_cannot_reenter() {
    let root = workspace_root();
    for retired in [
        "crates/engine-renderer/src/render_graph.rs",
        "crates/engine-renderer/src/material_resolver.rs",
        "crates/render-vulkan/src/renderer.rs",
        "crates/render-vulkan/src/frame.rs",
        "crates/render-vulkan/src/pipeline.rs",
        "crates/render-vulkan/src/passes",
        "crates/render-vulkan/src/resource.rs",
        "crates/render-vulkan/src/reload",
        "crates/render-vulkan/src/device_impl/pipeline.rs",
        "crates/render-vulkan/src/device_impl/rendering.rs",
        "crates/engine-editor/src/hot_reload_ui.rs",
        "crates/engine-editor/src/prefab_editor.rs",
        "crates/engine-editor/src/shader_watcher.rs",
        "crates/engine-asset/src/hot_reload.rs",
        "crates/sandbox/src/model_viewer.rs",
        "crates/engine-character/examples/character_demo.rs",
    ] {
        assert!(
            !root.join(retired).exists(),
            "retired architecture path must stay deleted: {retired}"
        );
    }

    let crates = root.join("crates");
    let mut sources = Vec::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let source_dir = entry.expect("crate directory entry").path().join("src");
        if source_dir.is_dir() {
            visit_rust_sources(&source_dir, &mut sources);
        }
    }
    let forbidden = [
        "render_model_frame",
        "render_triangle_frame",
        "VulkanRenderer",
        "to_legacy(",
        "extract_renderer_input_from_scene",
        "load_scene_to_world",
        "load_mesh_from_gltf",
        "load_meshes_from_gltf",
        "cook_orchestrate_unchecked",
        "pub fn cook_orchestrate(",
        "run_engine_character_demo",
        "engine-character-demo",
        "mesh_upload_from_data",
        "load_cooked_render_assets",
        "apply_gizmo_drag",
        "begin_gizmo_session",
        "end_gizmo_session",
        "HotReload",
        "DebouncedWatcher",
    ];
    for source in sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        for symbol in forbidden {
            assert!(
                !text.contains(symbol),
                "retired architecture symbol '{symbol}' re-entered {}",
                source.display()
            );
        }
    }
}

#[test]
fn react_layout_has_one_project_persisted_authority() {
    let root = workspace_root();
    let dock_layout =
        fs::read_to_string(root.join("crates/sandbox/editor-web/src/layout/dockLayout.ts"))
            .expect("React dock layout source");
    assert!(
        !dock_layout.contains("localStorage"),
        "React dock layout must restore from the project snapshot, not global browser storage"
    );
    assert!(dock_layout.contains("projectLayout?: string"));
    assert!(dock_layout.contains("engineBridge.invoke('layout.persist'"));

    let protocol = fs::read_to_string(root.join("crates/sandbox/src/editor_app/protocol.rs"))
        .expect("editor protocol source");
    assert!(!protocol.contains("\"layout.reset\""));
    assert!(protocol.contains("UI_OPEN_PANEL_EVENT"));

    let snapshot = fs::read_to_string(root.join("crates/sandbox/src/editor_app/snapshot.rs"))
        .expect("editor snapshot source");
    assert!(snapshot.contains("pub react_layout: String"));
    for retired in [
        "pub bottom_panel:",
        "pub show_hierarchy:",
        "pub show_inspector:",
        "pub show_bottom_panel:",
        "pub hierarchy_width:",
        "pub inspector_width:",
        "pub bottom_height:",
        "pub viewport_rect:",
    ] {
        assert!(
            !snapshot.contains(retired),
            "retired workspace authority '{retired}' re-entered the project snapshot"
        );
    }

    let editor_app = read_module_tree(&root.join("crates/sandbox/src/editor_app.rs"));
    for retired in [
        "enum BottomPanelTab",
        "struct EditorWorkspace {",
        "show_bottom_panel",
    ] {
        assert!(
            !editor_app.contains(retired),
            "retired native layout authority '{retired}' re-entered the editor"
        );
    }
}
