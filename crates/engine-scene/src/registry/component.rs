use std::collections::BTreeMap;

use engine_serialize::Value;

use crate::component::{Component, ComponentStorageDyn, SparseSet};
use crate::components::{Bounds, Camera, Light, Name, Renderable, Transform};

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// Metadata about a registered component type.
#[derive(Clone, Debug)]
pub struct ComponentMeta {
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub schema_version: (u16, u16, u16),
    pub has_editor: bool,
    pub has_script_binding: bool,
}

// ---------------------------------------------------------------------------
// Type aliases for extension hooks
// ---------------------------------------------------------------------------

/// Storage factory: creates a new `SparseSet` for this component type.
pub type StorageFactory = fn() -> Box<dyn ComponentStorageDyn>;

/// Serialization hook: convert component fields to a `BTreeMap<String, Value>`.
pub type SerializeFn = fn(&dyn std::any::Any) -> BTreeMap<String, Value>;

/// Deserialization hook: build a component from a `BTreeMap<String, Value>`.
pub type DeserializeFn = fn(&BTreeMap<String, Value>) -> Box<dyn std::any::Any>;

// ---------------------------------------------------------------------------
// ComponentExtension
// ---------------------------------------------------------------------------

/// A registered component extension.
pub struct ComponentExtension {
    pub meta: ComponentMeta,
    pub storage_factory: StorageFactory,
    pub serialize: Option<SerializeFn>,
    pub deserialize: Option<DeserializeFn>,
}

impl std::fmt::Debug for ComponentExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComponentExtension")
            .field("meta", &self.meta)
            .field("storage_factory", &"(fn)")
            .field("serialize", &self.serialize.map(|_| "(fn)"))
            .field("deserialize", &self.deserialize.map(|_| "(fn)"))
            .finish()
    }
}

// ---------------------------------------------------------------------------
// ComponentRegistry
// ---------------------------------------------------------------------------

/// Central registry for component types.
///
/// Allows subsystems (physics, animation, UI, audio, …) to register their own
/// component types without modifying core `engine-scene` sources.
pub struct ComponentRegistry {
    extensions: BTreeMap<&'static str, ComponentExtension>,
    order: Vec<&'static str>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            extensions: BTreeMap::new(),
            order: Vec::new(),
        }
    }

    /// Register a new component type.
    ///
    /// Returns `Err` with the type ID if a component with the same
    /// `type_id` is already registered.
    pub fn register(&mut self, ext: ComponentExtension) -> Result<(), &'static str> {
        let type_id = ext.meta.type_id;
        if self.extensions.contains_key(type_id) {
            return Err(type_id);
        }
        self.extensions.insert(type_id, ext);
        self.order.push(type_id);
        Ok(())
    }

    /// Check if a component type is registered.
    pub fn is_registered(&self, type_id: &str) -> bool {
        self.extensions.contains_key(type_id)
    }

    /// Get extension metadata by type ID.
    pub fn get(&self, type_id: &str) -> Option<&ComponentExtension> {
        self.extensions.get(type_id)
    }

    /// Create storage for all registered types (used by `World` initialization).
    pub fn create_storages(&self) -> BTreeMap<&'static str, Box<dyn ComponentStorageDyn>> {
        let mut storages = BTreeMap::new();
        for type_id in &self.order {
            if let Some(ext) = self.extensions.get(type_id) {
                storages.insert(*type_id, (ext.storage_factory)());
            }
        }
        storages
    }

    /// Register the six core engine components: [`Name`], [`Transform`],
    /// [`Renderable`], [`Camera`], [`Light`], [`Bounds`].
    pub fn register_core(&mut self) {
        macro_rules! core_ext {
            ($ty:ty, $display:expr, $has_editor:expr, $has_script:expr) => {{
                let ext = ComponentExtension {
                    meta: ComponentMeta {
                        type_id: <$ty as Component>::TYPE_ID,
                        display_name: $display,
                        schema_version: (0, 1, 0),
                        has_editor: $has_editor,
                        has_script_binding: $has_script,
                    },
                    storage_factory: || -> Box<dyn ComponentStorageDyn> {
                        Box::new(SparseSet::<$ty>::new())
                    },
                    serialize: None,
                    deserialize: None,
                };
                // Unwrap: core components are registered only once.
                self.register(ext).ok();
            }};
        }

        core_ext!(Name, "Name", true, false);
        core_ext!(Transform, "Transform", true, false);
        core_ext!(Renderable, "Renderable", true, false);
        core_ext!(Camera, "Camera", true, false);
        core_ext!(Light, "Light", true, false);
        core_ext!(Bounds, "Bounds", true, false);
    }

    /// Iterate over all registered extensions in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &ComponentExtension> {
        self.order
            .iter()
            .filter_map(move |type_id| self.extensions.get(type_id))
    }
}

impl Default for ComponentRegistry {
    fn default() -> Self {
        Self::new()
    }
}
