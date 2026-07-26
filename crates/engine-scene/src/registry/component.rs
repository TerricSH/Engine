use std::collections::{BTreeMap, BTreeSet};

use engine_serialize::Value;

use crate::component::{Component, ComponentStorageDyn, SparseSet};
use crate::components::{Bounds, Camera, Interactable, Light, Name, Renderable, Transform};
use crate::prefab_instance::PrefabInstanceRef;

// ---------------------------------------------------------------------------
// Metadata
// ---------------------------------------------------------------------------

/// How gameplay scripts may access a registered component through the
/// generic `Components` API.
///
/// This is the target-shape access model for the script component bridge; the
/// legacy `has_script_binding` boolean maps onto it (`true` →
/// [`ScriptAccess::ReadWrite`], `false` → [`ScriptAccess::None`]). Access is
/// only ever *effective* when the registry entry also carries both scene
/// serde hooks — the same hooks the scene loader uses — so scripts and scene
/// files share one field layout per component.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ScriptAccess {
    /// Not accessible to scripts.
    #[default]
    None,
    /// Scripts may query field snapshots; writes are rejected with a
    /// read-only diagnostic.
    ReadOnly,
    /// Scripts may query field snapshots and merge-write fields.
    ReadWrite,
    /// Scripts reach the component through a dedicated, higher-fidelity API
    /// (Transform commands, retained UI canvas handles), never the generic
    /// `Components` API.
    DedicatedApi,
}

impl ScriptAccess {
    /// Legacy `has_script_binding` semantics: any non-[`None`](Self::None)
    /// access level means the component participates in script bindings
    /// somewhere (generic bridge or a dedicated API).
    pub fn has_script_binding(self) -> bool {
        !matches!(self, Self::None)
    }

    /// Whether the generic `Components` API may query this component, given
    /// that the required serde hooks are present.
    pub fn is_queryable(self) -> bool {
        matches!(self, Self::ReadOnly | Self::ReadWrite)
    }

    /// Whether the generic `Components` API may write this component, given
    /// that the required serde hooks are present.
    pub fn is_writable(self) -> bool {
        matches!(self, Self::ReadWrite)
    }
}

/// Metadata about a registered component type.
#[derive(Clone, Debug)]
pub struct ComponentMeta {
    pub type_id: &'static str,
    pub display_name: &'static str,
    pub schema_version: (u16, u16, u16),
    pub has_editor: bool,
    pub script_access: ScriptAccess,
}

impl ComponentMeta {
    /// Legacy `has_script_binding` view of [`ComponentMeta::script_access`]:
    /// `true` for any non-[`ScriptAccess::None`] level.
    pub fn has_script_binding(&self) -> bool {
        self.script_access.has_script_binding()
    }
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

/// Optional semantic validation hook for the serialized field map.
pub type ValidateFieldsFn = fn(&BTreeMap<String, Value>) -> Result<(), String>;

// ---------------------------------------------------------------------------
// ComponentExtension
// ---------------------------------------------------------------------------

/// A registered component extension.
#[derive(Clone)]
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
#[derive(Clone, Debug)]
pub struct ComponentRegistry {
    extensions: BTreeMap<&'static str, ComponentExtension>,
    field_validators: BTreeMap<&'static str, ValidateFieldsFn>,
    singleton_types: BTreeSet<&'static str>,
    order: Vec<&'static str>,
}

impl ComponentRegistry {
    pub fn new() -> Self {
        Self {
            extensions: BTreeMap::new(),
            field_validators: BTreeMap::new(),
            singleton_types: BTreeSet::new(),
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

    /// Install semantic validation for a registered component's serialized
    /// field map. Scene loading, editor authoring, and generic script writes
    /// all consult this same hook before deserialization.
    pub fn register_fields_validator(
        &mut self,
        type_id: &'static str,
        validator: ValidateFieldsFn,
    ) -> Result<(), &'static str> {
        if !self.extensions.contains_key(type_id) || self.field_validators.contains_key(type_id) {
            return Err(type_id);
        }
        self.field_validators.insert(type_id, validator);
        Ok(())
    }

    /// Validate fields when the component registered a semantic validator.
    pub fn validate_fields(
        &self,
        type_id: &str,
        fields: &BTreeMap<String, Value>,
    ) -> Result<(), String> {
        self.field_validators
            .get(type_id)
            .map_or(Ok(()), |validator| validator(fields))
    }

    /// Mark a registered component type as scene-global singleton data.
    pub fn register_singleton(&mut self, type_id: &'static str) -> Result<(), &'static str> {
        if !self.extensions.contains_key(type_id) || !self.singleton_types.insert(type_id) {
            return Err(type_id);
        }
        Ok(())
    }

    pub fn singleton_types(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.singleton_types.iter().copied()
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

    /// Register the core engine components, including serialized prefab
    /// instance linkage.
    pub fn register_core(&mut self) {
        macro_rules! core_ext {
            ($ty:ty, $display:expr, $has_editor:expr, $access:expr) => {{
                let ext = ComponentExtension {
                    meta: ComponentMeta {
                        type_id: <$ty as Component>::TYPE_ID,
                        display_name: $display,
                        schema_version: (0, 1, 0),
                        has_editor: $has_editor,
                        script_access: $access,
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
            ($ty:ty, $display:expr, $has_editor:expr, $access:expr, $ser:expr, $de:expr) => {{
                let ext = ComponentExtension {
                    meta: ComponentMeta {
                        type_id: <$ty as Component>::TYPE_ID,
                        display_name: $display,
                        schema_version: (0, 1, 0),
                        has_editor: $has_editor,
                        script_access: $access,
                    },
                    storage_factory: || -> Box<dyn ComponentStorageDyn> {
                        Box::new(SparseSet::<$ty>::new())
                    },
                    serialize: Some($ser),
                    deserialize: Some($de),
                };
                // Unwrap: core components are registered only once.
                self.register(ext).ok();
            }};
        }

        core_ext!(Name, "Name", true, ScriptAccess::None);
        core_ext!(Transform, "Transform", true, ScriptAccess::DedicatedApi);
        core_ext!(Renderable, "Renderable", true, ScriptAccess::None);
        core_ext!(
            Camera,
            "Camera",
            true,
            ScriptAccess::ReadWrite,
            crate::components::serialize_camera,
            crate::components::deserialize_camera
        );
        core_ext!(
            Light,
            "Light",
            true,
            ScriptAccess::ReadWrite,
            crate::components::serialize_light,
            crate::components::deserialize_light
        );
        core_ext!(
            Interactable,
            "Interactable",
            true,
            ScriptAccess::ReadWrite,
            crate::components::serialize_interactable,
            crate::components::deserialize_interactable
        );
        self.register_fields_validator(
            Interactable::TYPE_ID,
            crate::components::validate_interactable_fields,
        )
        .ok();
        core_ext!(Bounds, "Bounds", true, ScriptAccess::None);
        core_ext!(
            PrefabInstanceRef,
            "Prefab Instance",
            false,
            ScriptAccess::None
        );
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
