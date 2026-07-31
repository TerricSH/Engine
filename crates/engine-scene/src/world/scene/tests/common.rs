    use std::any::Any;
    use std::collections::BTreeMap;
    use std::sync::Arc;

    use crate::components::{Camera, Name, Renderable, Transform};
    use crate::registry::{ComponentExtension, ComponentMeta, ComponentRegistry};
    use crate::scene::{sample_scene, ComponentRecord, SceneLoadDiagnostic};
    use crate::{Component, ComponentStorageDyn, PrefabInstanceRef, SparseSet, World};
    use engine_serialize::{AssetId, SchemaVersion, Value};

    #[derive(Debug, PartialEq)]
    struct ExternalComponent {
        value: u64,
    }

    impl Component for ExternalComponent {
        const TYPE_ID: &'static str = "test.external";
    }

    struct WrongComponent;

    impl Component for WrongComponent {
        const TYPE_ID: &'static str = "test.wrong";
    }

    fn external_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<ExternalComponent>::new())
    }

    fn wrong_storage() -> Box<dyn ComponentStorageDyn> {
        Box::new(SparseSet::<WrongComponent>::new())
    }

    fn serialize_external(component: &dyn Any) -> BTreeMap<String, Value> {
        let component = component
            .downcast_ref::<ExternalComponent>()
            .expect("external component type");
        BTreeMap::from([("value".to_string(), Value::UInt(component.value))])
    }

    fn deserialize_external(fields: &BTreeMap<String, Value>) -> Box<dyn Any> {
        let value = match fields.get("value") {
            Some(Value::UInt(value)) => *value,
            _ => 0,
        };
        Box::new(ExternalComponent { value })
    }

    fn deserialize_wrong_type(_: &BTreeMap<String, Value>) -> Box<dyn Any> {
        Box::new(WrongComponent)
    }

    fn external_extension(
        deserialize: Option<crate::registry::DeserializeFn>,
    ) -> ComponentExtension {
        ComponentExtension {
            meta: ComponentMeta {
                type_id: ExternalComponent::TYPE_ID,
                display_name: "External",
                schema_version: (0, 1, 0),
                has_editor: false,
                script_access: crate::registry::ScriptAccess::None,
            },
            storage_factory: external_storage,
            serialize: Some(serialize_external),
            deserialize,
        }
    }

    fn scene_with_component(type_id: &str) -> crate::Scene {
        let mut scene = sample_scene();
        scene.entities[0].components.insert(
            type_id.to_string(),
            ComponentRecord {
                schema_version: SchemaVersion::new(0, 1, 0),
                enabled: true,
                fields: BTreeMap::from([("value".to_string(), Value::UInt(42))]),
            },
        );
        scene
    }
