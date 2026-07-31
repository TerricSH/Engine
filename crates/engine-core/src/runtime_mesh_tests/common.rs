use super::*;
use engine_renderer::{BackendRenderer, FrameStats, ResourceRemoval};
use std::sync::{Arc, Mutex};

fn triangle() -> RuntimeMeshDescriptor {
    RuntimeMeshDescriptor {
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        normals: vec![Vec3::Z; 3],
        uvs: vec![Vec2::ZERO; 3],
        indices: vec![0, 1, 2],
        bounds: None,
    }
}

fn quad() -> RuntimeMeshDescriptor {
    RuntimeMeshDescriptor {
        positions: vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ],
        normals: vec![Vec3::Z; 4],
        uvs: vec![Vec2::ZERO; 4],
        indices: vec![0, 1, 2, 0, 2, 3],
        bounds: Some((Vec3::ZERO, Vec3::ONE)),
    }
}

fn runtime() -> EngineRuntime {
    EngineRuntime::new(crate::EngineConfig::default())
}

fn registered_upload(runtime: &EngineRuntime, handle: RuntimeMeshHandle) -> MeshUpload {
    let id = runtime
        .runtime_mesh_asset_id(handle)
        .expect("live handle resolves an asset ID");
    runtime
        .asset_registry()
        .get::<MeshUpload>(&id)
        .expect("runtime mesh is registered")
        .get()
        .clone()
}
