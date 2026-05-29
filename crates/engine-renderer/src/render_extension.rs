use crate::RenderFrameInput;

/// Produces additional render data each frame.
///
/// Extensions are called once per frame before the render graph executes,
/// allowing them to inject drawables, skinned items, debug primitives, or
/// other render data into the frame input.
pub trait RenderExtensionProducer: Send {
    /// Human-readable name of this producer (for diagnostics).
    fn name(&self) -> &str;

    /// Populate the frame input with additional render data.
    ///
    /// Called once per frame, before the render graph processes the input.
    fn produce(&self, input: &mut RenderFrameInput, frame_index: u64);
}

/// Registry for render extensions.
pub struct RenderExtensionRegistry {
    producers: Vec<Box<dyn RenderExtensionProducer>>,
}

impl RenderExtensionRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            producers: Vec::new(),
        }
    }

    /// Register a render extension producer.
    pub fn register(&mut self, producer: Box<dyn RenderExtensionProducer>) {
        self.producers.push(producer);
    }

    /// Call `produce` on every registered producer.
    pub fn produce_all(&self, input: &mut RenderFrameInput, frame_index: u64) {
        for producer in &self.producers {
            producer.produce(input, frame_index);
        }
    }

    /// Returns the number of registered producers.
    pub fn producer_count(&self) -> usize {
        self.producers.len()
    }
}

impl Default for RenderExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ══════════════════════════════════════════════════════════════════════════
// Tests
// ══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AxisAlignedBox, SkinnedItem, IDENTITY_MAT4};

    /// A dummy producer that adds a skinned item.
    struct DummySkinnedProducer;

    impl RenderExtensionProducer for DummySkinnedProducer {
        fn name(&self) -> &str {
            "DummySkinnedProducer"
        }

        fn produce(&self, input: &mut RenderFrameInput, _frame_index: u64) {
            input.skinned_items.push(SkinnedItem {
                entity: None,
                mesh: crate::AssetId::new("mesh_dummy"),
                material: crate::AssetId::new("mat_dummy"),
                skeleton: crate::AssetId::new("skeleton_dummy"),
                bone_palette: vec![IDENTITY_MAT4; 4],
                bone_palette_layout: crate::BonePaletteLayout::Full4x4 { count: 4 },
                world_transform: IDENTITY_MAT4,
                bounds: AxisAlignedBox {
                    min: [-1.0, -1.0, -1.0],
                    max: [1.0, 1.0, 1.0],
                },
                render_layer: "default".to_string(),
                cast_shadows: true,
                sort_key: 0,
            });
        }
    }

    #[test]
    fn registry_new_is_empty() {
        let reg = RenderExtensionRegistry::new();
        assert_eq!(reg.producer_count(), 0);
    }

    #[test]
    fn registry_register_increases_count() {
        let mut reg = RenderExtensionRegistry::new();
        reg.register(Box::new(DummySkinnedProducer));
        assert_eq!(reg.producer_count(), 1);
    }

    #[test]
    fn produce_all_calls_all_producers() {
        let mut reg = RenderExtensionRegistry::new();
        reg.register(Box::new(DummySkinnedProducer));
        reg.register(Box::new(DummySkinnedProducer));

        let mut input = RenderFrameInput::empty(42);
        reg.produce_all(&mut input, 42);

        assert_eq!(input.skinned_items.len(), 2);
    }

    #[test]
    fn produce_all_with_no_producers() {
        let reg = RenderExtensionRegistry::new();
        let mut input = RenderFrameInput::empty(0);
        reg.produce_all(&mut input, 0);
        assert_eq!(input.skinned_items.len(), 0);
    }

    #[test]
    fn producer_name_is_reported() {
        let producer = DummySkinnedProducer;
        assert_eq!(producer.name(), "DummySkinnedProducer");
    }

    /// A producer that injects debug primitives.
    struct DebugPrimitiveProducer;

    impl RenderExtensionProducer for DebugPrimitiveProducer {
        fn name(&self) -> &str {
            "DebugPrimitiveProducer"
        }

        fn produce(&self, input: &mut RenderFrameInput, _frame_index: u64) {
            input.debug_primitives.push(crate::DebugPrimitive {
                source_system: "test".to_string(),
                severity: crate::DiagnosticSeverity::Info,
                primitive_kind: crate::DebugPrimitiveKind::Line {
                    from: [0.0, 0.0, 0.0],
                    to: [1.0, 0.0, 0.0],
                },
                color: [1.0, 0.0, 0.0, 1.0],
                lifetime_frames: 1,
            });
        }
    }

    #[test]
    fn produce_all_injects_primitives() {
        let mut reg = RenderExtensionRegistry::new();
        reg.register(Box::new(DebugPrimitiveProducer));

        let mut input = RenderFrameInput::empty(0);
        reg.produce_all(&mut input, 0);

        assert_eq!(input.debug_primitives.len(), 1);
    }
}
