//! Core ECS component types for the engine.

mod bounds;
mod camera;
mod hlod_cluster;
mod interactable;
mod light;
mod lod_group;
mod name;
mod renderable;
mod transform;
mod triplanar_material_mapping;
mod vertex_geomorph;

pub use bounds::Bounds;
pub use camera::{
    deserialize_camera, deserialize_camera_fields, serialize_camera, serialize_camera_fields,
    Camera, CameraProjection,
};
pub use hlod_cluster::{
    deserialize_hlod_cluster, deserialize_hlod_cluster_fields, serialize_hlod_cluster,
    serialize_hlod_cluster_fields, validate_hlod_cluster_fields, HlodCluster, HlodRole,
};
pub use interactable::{
    deserialize_interactable, deserialize_interactable_fields, serialize_interactable,
    serialize_interactable_fields, validate_interactable_fields, Interactable,
};
pub use light::{
    deserialize_light, deserialize_light_fields, serialize_light, serialize_light_fields, Light,
    LightKind,
};
pub use lod_group::{
    deserialize_lod_group, deserialize_lod_group_fields, serialize_lod_group,
    serialize_lod_group_fields, validate_lod_group_fields, LodGroup, LodLevel,
};
pub use name::Name;
pub use renderable::{
    deserialize_renderable, deserialize_renderable_fields, serialize_renderable,
    serialize_renderable_fields, validate_renderable_fields, Renderable,
};
pub use transform::Transform;
pub use triplanar_material_mapping::TriplanarMaterialMapping;
pub use vertex_geomorph::VertexGeomorph;

/// Extract an f32 from a scene [`engine_serialize::Value`], defaulting to 0.0.
///
/// Accepts every numeric scene representation so field maps authored by hand
/// or migrated between schema versions stay loadable.
pub(crate) fn field_as_f32(value: &engine_serialize::Value) -> f32 {
    use engine_serialize::Value;
    match value {
        Value::Float32(v) => *v,
        Value::Float64(v) => *v as f32,
        Value::Int(v) => *v as f32,
        Value::UInt(v) => *v as f32,
        _ => 0.0,
    }
}
