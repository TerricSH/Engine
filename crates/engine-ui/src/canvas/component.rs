use super::serialization::deserialize_canvas;
use super::*;

// ECS Component
// ---------------------------------------------------------------------------

impl engine_scene::Component for Canvas {
    const TYPE_ID: &'static str = "engine.canvas";
}

// ---------------------------------------------------------------------------
// ECS registration
// ---------------------------------------------------------------------------

/// Register UI extensions (Canvas component) with the engine's component
/// registry.
///
/// Call this during engine initialisation so that the Canvas component
/// type is recognised by the ECS world.
pub fn register_ui_extensions(component_registry: &mut engine_scene::registry::ComponentRegistry) {
    use engine_scene::registry::{ComponentExtension, ComponentMeta};
    use engine_scene::{Component, ComponentStorageDyn, SparseSet};

    let _ = component_registry.register(ComponentExtension {
        meta: ComponentMeta {
            type_id: Canvas::TYPE_ID,
            display_name: "UI Canvas",
            schema_version: (0, 1, 0),
            has_editor: true,
            // Scripts drive canvases through the retained `UICanvas` managed
            // handles, never the generic Components bridge.
            script_access: engine_scene::registry::ScriptAccess::DedicatedApi,
        },
        storage_factory: || -> Box<dyn ComponentStorageDyn> {
            Box::new(SparseSet::<Canvas>::new())
        },
        serialize: Some(serialize_canvas),
        deserialize: Some(deserialize_canvas),
    });
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------
