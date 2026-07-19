//! Architecture guards for the game-script / engine-runtime dependency line.

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("sandbox must live under <workspace>/crates")
        .to_path_buf()
}

fn visit_rust_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
        .map(|entry| entry.expect("source directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            visit_rust_sources(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

#[test]
fn engine_crates_do_not_depend_on_the_sandbox_application() {
    let crates = workspace_root().join("crates");
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let crate_dir = entry.expect("crate directory entry").path();
        if !crate_dir.is_dir()
            || crate_dir.file_name().and_then(|name| name.to_str()) == Some("sandbox")
        {
            continue;
        }
        let manifest = crate_dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest).expect("crate manifest");
        assert!(
            !text.lines().any(|line| {
                let compact = line.trim_start();
                compact.starts_with("sandbox =") || compact.contains("path = \"../sandbox\"")
            }),
            "engine crate {} must not depend on the sandbox application",
            crate_dir.display()
        );
    }
}

#[test]
fn production_engine_sources_do_not_reference_example_game_content() {
    let crates = workspace_root().join("crates");
    let mut sources = Vec::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let source_dir = entry.expect("crate directory entry").path().join("src");
        if source_dir.is_dir() {
            visit_rust_sources(&source_dir, &mut sources);
        }
    }

    for source in sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        assert!(
            !text.contains("examples/minimal-game") && !text.contains(r"examples\minimal-game"),
            "production source {} must consume a project path, not the repository example game",
            source.display()
        );
    }
}

#[test]
fn script_api_contract_crate_remains_data_only() {
    let manifest = workspace_root().join("crates/engine-script-api/Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("engine-script-api manifest");
    let dependencies = text
        .split_once("[dependencies]")
        .map(|(_, value)| value.trim())
        .unwrap_or_default();
    assert!(
        dependencies.is_empty(),
        "engine-script-api must not acquire runtime, renderer, ECS, editor, or platform dependencies"
    );
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
        "to_legacy",
        "extract_renderer_input_from_scene",
        "load_scene_to_world",
        "load_mesh_from_gltf",
        "load_meshes_from_gltf",
        "cook_orchestrate_unchecked",
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

    let editor_app = fs::read_to_string(root.join("crates/sandbox/src/editor_app.rs"))
        .expect("editor application source");
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

#[test]
fn retired_placeholder_launch_command_cannot_reenter() {
    let main = fs::read_to_string(workspace_root().join("crates/sandbox/src/main.rs"))
        .expect("sandbox main source");
    assert!(
        !main.contains("\"workspace\" =>"),
        "the retired placeholder launch command must stay deleted"
    );
}

#[test]
fn production_editor_asset_and_vulkan_paths_cannot_hide_dead_implementations() {
    let root = workspace_root();
    for directory in [
        "crates/engine-asset/src",
        "crates/engine-editor/src",
        "crates/engine-ui/src",
        "crates/render-vulkan/src",
        "crates/sandbox/src",
    ] {
        let mut sources = Vec::new();
        visit_rust_sources(&root.join(directory), &mut sources);
        for source in sources {
            let text = fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
            for attribute in ["#[allow(dead_code)]", "#[expect(dead_code)]"] {
                assert!(
                    !text.contains(attribute),
                    "production source {} hides an unused implementation with {attribute}",
                    source.display()
                );
            }
        }
    }
}

#[test]
fn production_material_editor_cannot_inject_test_fallback_parameters() {
    let source =
        fs::read_to_string(workspace_root().join("crates/engine-editor/src/material_editor.rs"))
            .expect("material editor source");
    for retired_marker in [
        "Keep an editable fallback for assets",
        "v0 injects 6 synthetic parameters",
        "Both loads produce the same synthetic params",
    ] {
        assert!(
            !source.contains(retired_marker),
            "retired production material test fallback re-entered: {retired_marker}"
        );
    }
}

#[test]
fn editor_scene_viewport_cannot_fall_back_to_fake_overlay_or_full_surface_rendering() {
    let root = workspace_root();
    assert!(
        !root
            .join("crates/sandbox/src/editor_app/egui_editor.rs")
            .exists(),
        "the retired egui editor must stay physically deleted"
    );
    let editor_ui = fs::read_to_string(root.join("crates/sandbox/editor-web/src/App.tsx"))
        .expect("React editor source");
    for retired in ["paint_scene_grid", "paint_orientation_axes", "SetShowGrid"] {
        assert!(
            !editor_ui.contains(retired),
            "fake Scene viewport decoration '{retired}' must stay deleted"
        );
    }

    let editor_app = fs::read_to_string(root.join("crates/sandbox/src/editor_app.rs"))
        .expect("editor application source");
    assert!(editor_app.contains("render_embedded_viewport"));
    assert!(editor_app.contains("extract_renderer_input_from_world_with_viewport"));
    assert!(!editor_app.contains("render_egui"));
    assert!(!editor_ui.contains("WebGL"));

    let vulkan = fs::read_to_string(root.join("crates/render-vulkan/src/scene_renderer.rs"))
        .expect("Vulkan scene renderer source");
    assert!(!vulkan.contains("supports only a full-surface viewport"));
    assert!(!vulkan.contains("#[allow(dead_code)]\nstruct BonePaletteCacheEntry"));
}

#[test]
fn render_graph_has_one_canonical_compiler() {
    let root = workspace_root();
    let source = fs::read_to_string(root.join("crates/engine-renderer/src/render_graph2.rs"))
        .expect("canonical render graph source");
    assert!(
        !source.contains("compile_v2"),
        "the retired second render-graph compiler must stay deleted"
    );
    assert_eq!(
        source.matches("pub fn compile(&self)").count(),
        1,
        "RenderGraph must expose exactly one compiler"
    );
}

#[test]
fn react_editor_bridge_has_one_transport_and_one_receive_entry() {
    let root = workspace_root();
    let bridge =
        fs::read_to_string(root.join("crates/sandbox/editor-web/src/bridge/engineBridge.ts"))
            .expect("React bridge source");
    assert!(
        bridge.contains("window.ipc"),
        "the React bridge must use Wry's canonical window.ipc transport"
    );
    assert!(
        bridge.contains("window.__ENGINE_EDITOR_RECEIVE__ = deliver"),
        "the React bridge must expose the one native receive callback"
    );
    for retired in ["__ENGINE_IPC__", "engine-message"] {
        assert!(
            !bridge.contains(retired),
            "retired React bridge compatibility entry '{retired}' must stay deleted"
        );
    }
}
