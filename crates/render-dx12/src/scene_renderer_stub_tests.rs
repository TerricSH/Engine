use super::*;
use engine_renderer::{
    AssetId, AxisAlignedBox, BackendRenderer, IndexFormat, MeshUpload, MeshVertexFormat,
    RenderFrameInput, PBR32_VERTEX_STRIDE,
};

#[test]
fn unavailable_backend_never_reports_render_upload_or_resize_success() {
    let mut renderer = Dx12SceneRenderer;
    assert!(renderer.begin_frame(&RenderFrameInput::empty(0)).is_err());
    assert!(renderer
        .upload_mesh(MeshUpload {
            mesh_id: AssetId::new("mesh"),
            vertex_format: MeshVertexFormat::Pbr32,
            vertex_count: 1,
            vertex_bytes: vec![0; PBR32_VERTEX_STRIDE as usize],
            index_format: IndexFormat::U16,
            index_count: 3,
            index_bytes: vec![0; 6],
            bounds: AxisAlignedBox::UNIT,
            content_hash: [1; 32],
        })
        .is_err());
    assert!(renderer.resize(1280, 720).is_err());
    assert!(renderer.resize(0, 720).is_err());
}
