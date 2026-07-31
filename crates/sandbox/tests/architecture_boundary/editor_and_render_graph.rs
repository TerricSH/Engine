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

    let editor_app = read_module_tree(&root.join("crates/sandbox/src/editor_app.rs"));
    assert!(editor_app.contains("render_embedded_viewport"));
    assert!(editor_app.contains("extract_renderer_input_from_world_with_viewport"));
    assert!(!editor_app.contains("render_egui"));
    assert!(!editor_ui.contains("WebGL"));

    let vulkan = read_module_tree(&root.join("crates/render-vulkan/src/scene_renderer.rs"));
    assert!(!vulkan.contains("supports only a full-surface viewport"));
    assert!(!vulkan.contains("#[allow(dead_code)]\nstruct BonePaletteCacheEntry"));
}

#[test]
fn render_graph_has_one_canonical_compiler() {
    let root = workspace_root();
    let source = read_module_tree(&root.join("crates/engine-renderer/src/render_graph2.rs"));
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
fn registry_sync_is_the_only_engine_core_backend_resource_owner() {
    let root = workspace_root();
    let runtime_state = fs::read_to_string(root.join("crates/engine-core/src/runtime/state.rs"))
        .expect("engine runtime state");
    let rendering = fs::read_to_string(root.join("crates/engine-core/src/runtime/rendering.rs"))
        .expect("engine runtime rendering facade");
    assert!(rendering.contains("fn remove_unregistered_render_assets"));
    assert_eq!(
        rendering.matches("self.renderer.remove_resource(").count(),
        1,
        "EngineRuntime must reconcile every backend removal in the canonical registry sync"
    );

    let runtime_mesh = fs::read_to_string(root.join("crates/engine-core/src/runtime_mesh.rs"))
        .expect("runtime mesh source");
    let production = runtime_mesh
        .split("#[cfg(test)]")
        .next()
        .expect("runtime mesh production source");
    for retired in ["pending_gpu_removals", "self.renderer", "ResourceRemoval"] {
        assert!(
            !production.contains(retired),
            "runtime meshes must use AssetRegistry synchronization, not '{retired}'"
        );
    }
    for retired in [
        "upload_temporary_preview_texture",
        "remove_temporary_preview_texture",
        "temporary_preview_texture_ids",
    ] {
        assert!(
            !rendering.contains(retired),
            "temporary preview resources must not expose the retired backend-style API '{retired}'"
        );
    }
    assert!(
        runtime_state.contains("temporary_preview_textures")
            && rendering.contains("Arc::ptr_eq(&owned.shared(), &current.shared())"),
        "temporary preview ownership must follow the exact registry allocation, not only AssetId"
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
