use crate::render_graph2::{CompiledBarrier, PassNode};
use crate::{
    validate_frame_input, AssetId, Diagnostic, DiagnosticSeverity, FrameStats, MaterialUpload,
    MeshUpload, RenderFrameInput, ResourceRemoval, TextureUpload, Transparency, UploadReceipt,
};

pub const DIAG_BACKEND_MISSING: &str = "RV0100";
pub const DIAG_MESH_UPLOAD_UNSUPPORTED: &str = "RV0101";
pub const DIAG_TEXTURE_UPLOAD_UNSUPPORTED: &str = "RV0102";
pub const DIAG_MATERIAL_UPLOAD_UNSUPPORTED: &str = "RV0103";
pub const DIAG_RESOURCE_REMOVAL_UNSUPPORTED: &str = "RV0104";
pub const DIAG_RESIZE_UNSUPPORTED: &str = "RV0105";
pub const DIAG_ABORT_UNSUPPORTED: &str = "RV0106";
pub const DIAG_INVALID_RESOURCE_ID: &str = "RV0110";
pub const DIAG_INVALID_MESH_VERTICES: &str = "RV0111";
pub const DIAG_INVALID_MESH_INDICES: &str = "RV0112";
pub const DIAG_INVALID_MESH_BOUNDS: &str = "RV0113";
pub const DIAG_INVALID_TEXTURE_DIMENSIONS: &str = "RV0120";
pub const DIAG_INVALID_TEXTURE_MIPS: &str = "RV0121";
pub const DIAG_INVALID_MATERIAL_VALUES: &str = "RV0130";
pub const DIAG_UNSUPPORTED_MATERIAL_STATE: &str = "RV0131";
pub const DIAG_INVALID_RESIZE: &str = "RV0140";
pub const DIAG_BARRIERS_UNSUPPORTED: &str = "RV0141";
pub const DIAG_RENDER_GRAPH_UNSUPPORTED: &str = "RV0142";
pub const DIAG_CUSTOM_RENDER_GRAPH_UNSUPPORTED: &str = "RV0143";

/// Backend renderer trait implemented by concrete GPU backends.
pub trait BackendRenderer: Send {
    /// Let a graph-capable backend replace frontend placeholder declarations
    /// for its registered custom passes before dependency compilation.
    ///
    /// Built-in-only graphs need no backend participation. A custom node is
    /// rejected by default so its declared resources cannot be silently
    /// treated as empty by a backend that has no pass registry.
    fn configure_render_graph(
        &mut self,
        _input: &RenderFrameInput,
        graph: &mut crate::render_graph2::RenderGraph,
    ) -> Result<(), Vec<Diagnostic>> {
        if let Some(custom) = graph.passes.iter().find_map(|pass| match &pass.kind {
            crate::render_graph2::PassKind::Custom(name) => Some(*name),
            _ => None,
        }) {
            Err(vec![Diagnostic::new(
                DIAG_CUSTOM_RENDER_GRAPH_UNSUPPORTED,
                DiagnosticSeverity::Error,
                "renderer.render_graph",
                format!(
                    "backend does not provide a resource declaration for custom render pass '{custom}'"
                ),
            )])
        } else {
            Ok(())
        }
    }

    /// Begin a new frame. Called once before [`execute_pass`](Self::execute_pass).
    fn begin_frame(&mut self, _input: &RenderFrameInput) -> Result<(), Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_RENDER_GRAPH_UNSUPPORTED,
            "render-graph begin_frame",
        ))
    }

    /// End the current frame after every compiled pass succeeds.
    fn end_frame(&mut self, _stats: &mut FrameStats) -> Result<(), Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_RENDER_GRAPH_UNSUPPORTED,
            "render-graph end_frame",
        ))
    }

    /// Abandon a frame that began successfully but subsequently failed.
    ///
    /// A backend must release or reset any transient recording state without
    /// presenting the incomplete frame. The default is deliberately an error:
    /// silently claiming an unknown backend was reset is unsafe.
    fn abort_frame(&mut self) -> Result<(), Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_ABORT_UNSUPPORTED,
            "frame abort",
        ))
    }

    /// Apply graph-compiled resource barriers before a pass executes.
    fn apply_pass_barriers(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &PassNode,
        barriers: &[CompiledBarrier],
    ) -> Result<(), Vec<Diagnostic>> {
        if barriers.is_empty() {
            Ok(())
        } else {
            Err(unsupported_backend_operation(
                DIAG_BARRIERS_UNSUPPORTED,
                "render-graph resource barriers",
            ))
        }
    }

    /// Execute a single render-graph pass.
    fn execute_pass(
        &mut self,
        _input: &RenderFrameInput,
        _pass: &PassNode,
        _frame_stats: &mut FrameStats,
    ) -> Result<(), Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_RENDER_GRAPH_UNSUPPORTED,
            "render-graph pass execution",
        ))
    }

    /// Upload one owned static mesh and return its backend revision.
    fn upload_mesh(&mut self, _upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_MESH_UPLOAD_UNSUPPORTED,
            "mesh upload",
        ))
    }

    /// Upload one owned 2D texture and all declared mip levels.
    fn upload_texture(&mut self, _upload: TextureUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_TEXTURE_UPLOAD_UNSUPPORTED,
            "texture upload",
        ))
    }

    /// Upload one owned metallic-roughness material.
    fn upload_material(
        &mut self,
        _upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_MATERIAL_UPLOAD_UNSUPPORTED,
            "material upload",
        ))
    }

    /// Remove a previously uploaded resource.
    fn remove_resource(&mut self, _removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_RESOURCE_REMOVAL_UNSUPPORTED,
            "resource removal",
        ))
    }

    /// Resize the underlying swapchain and viewport.
    fn resize(&mut self, _width: u32, _height: u32) -> Result<(), Vec<Diagnostic>> {
        Err(unsupported_backend_operation(
            DIAG_RESIZE_UNSUPPORTED,
            "surface resize",
        ))
    }

    /// Enable or disable GPU timestamp profiling (ENG-04).
    ///
    /// Backends with timestamp support (Vulkan) honour this switch; other
    /// backends keep reporting
    /// [`GpuTimingStatus::Unavailable`](crate::frame_timing::GpuTimingStatus::Unavailable)
    /// through their frame statistics. The default is a no-op.
    fn set_gpu_timing_enabled(&mut self, _enabled: bool) {}
}

pub struct Renderer {
    backend: Option<Box<dyn BackendRenderer>>,
}

impl Renderer {
    pub fn new() -> Self {
        Self { backend: None }
    }

    pub fn new_with_backend(backend: Box<dyn BackendRenderer>) -> Self {
        Self {
            backend: Some(backend),
        }
    }

    pub fn set_backend(&mut self, backend: Box<dyn BackendRenderer>) {
        self.backend = Some(backend);
    }

    /// Resize the active backend's swapchain and viewport.
    pub fn resize(&mut self, width: u32, height: u32) -> Result<(), Vec<Diagnostic>> {
        if width == 0 || height == 0 {
            return Err(vec![Diagnostic::new(
                DIAG_INVALID_RESIZE,
                DiagnosticSeverity::Error,
                "renderer.contract",
                format!("resize dimensions must be non-zero, got {width}x{height}"),
            )]);
        }
        self.backend
            .as_mut()
            .ok_or_else(|| missing_backend("resize"))?
            .resize(width, height)
    }

    /// Validate and upload one static PBR mesh.
    pub fn upload_mesh(&mut self, upload: MeshUpload) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let diagnostics = validate_mesh_upload(&upload);
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        self.backend
            .as_mut()
            .ok_or_else(|| missing_backend("mesh upload"))?
            .upload_mesh(upload)
    }

    /// Validate and upload one RGBA8 texture and its mip chain.
    pub fn upload_texture(
        &mut self,
        upload: TextureUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let diagnostics = validate_texture_upload(&upload);
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        self.backend
            .as_mut()
            .ok_or_else(|| missing_backend("texture upload"))?
            .upload_texture(upload)
    }

    /// Validate and upload one portable metallic-roughness material.
    pub fn upload_material(
        &mut self,
        upload: MaterialUpload,
    ) -> Result<UploadReceipt, Vec<Diagnostic>> {
        let diagnostics = validate_material_upload(&upload);
        if !diagnostics.is_empty() {
            return Err(diagnostics);
        }
        self.backend
            .as_mut()
            .ok_or_else(|| missing_backend("material upload"))?
            .upload_material(upload)
    }

    /// Remove a previously uploaded resource.
    pub fn remove_resource(&mut self, removal: ResourceRemoval) -> Result<(), Vec<Diagnostic>> {
        if let Some(diagnostic) = validate_resource_id(&removal.resource_id, "resource") {
            return Err(vec![diagnostic]);
        }
        self.backend
            .as_mut()
            .ok_or_else(|| missing_backend("resource removal"))?
            .remove_resource(removal)
    }

    /// Render a frame by building the render graph and executing each pass.
    pub fn draw_scene(&mut self, input: &RenderFrameInput) -> Result<FrameStats, Vec<Diagnostic>> {
        let diagnostics = validate_frame_input(input);
        if diagnostics.iter().any(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        }) {
            return Err(diagnostics);
        }

        let backend = self
            .backend
            .as_mut()
            .ok_or_else(|| missing_backend("draw"))?;

        let mut graph = crate::render_graph2::RenderGraph::build_with_config(
            input,
            &input.render_options.pass_graph_config,
        );
        backend.configure_render_graph(input, &mut graph)?;
        let compiled = graph.compile().map_err(|error| {
            vec![Diagnostic::new(
                "RV0020",
                DiagnosticSeverity::Error,
                "renderer.render_graph",
                format!("render graph compile failed: {error}"),
            )]
        })?;

        let mut stats = FrameStats::default();
        backend.begin_frame(input)?;

        for (compiled_index, &pass_index) in compiled.pass_order.iter().enumerate() {
            let Some(pass) = graph.passes.get(pass_index) else {
                let diagnostics = vec![Diagnostic::new(
                    "RV0021",
                    DiagnosticSeverity::Error,
                    "renderer.render_graph",
                    format!("compiled graph referenced missing pass index {pass_index}"),
                )];
                return Err(abort_after_failure(backend.as_mut(), diagnostics));
            };
            let barriers = compiled
                .barriers_per_pass
                .get(compiled_index)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let span = tracing::debug_span!("frame.view.{}.{}", input.frame_index, pass.name);
            let _guard = span.enter();
            tracing::debug!(pass = pass.name, "executing render pass");

            if let Err(diagnostics) = backend.apply_pass_barriers(input, pass, barriers) {
                return Err(abort_after_failure(backend.as_mut(), diagnostics));
            }
            if let Err(diagnostics) = backend.execute_pass(input, pass, &mut stats) {
                return Err(abort_after_failure(backend.as_mut(), diagnostics));
            }
        }

        if let Err(diagnostics) = backend.end_frame(&mut stats) {
            return Err(abort_after_failure(backend.as_mut(), diagnostics));
        }

        Ok(stats)
    }
}

impl Default for Renderer {
    fn default() -> Self {
        Self::new()
    }
}

fn unsupported_backend_operation(code: &str, operation: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::new(
        code,
        DiagnosticSeverity::Error,
        "renderer.backend",
        format!("backend does not implement {operation}"),
    )]
}

fn missing_backend(operation: &str) -> Vec<Diagnostic> {
    vec![Diagnostic::new(
        DIAG_BACKEND_MISSING,
        DiagnosticSeverity::Error,
        "renderer.backend",
        format!("cannot perform {operation}: no renderer backend is installed"),
    )]
}

fn abort_after_failure(
    backend: &mut dyn BackendRenderer,
    mut diagnostics: Vec<Diagnostic>,
) -> Vec<Diagnostic> {
    if let Err(mut abort_diagnostics) = backend.abort_frame() {
        diagnostics.append(&mut abort_diagnostics);
    }
    diagnostics
}

fn validate_resource_id(resource_id: &AssetId, label: &str) -> Option<Diagnostic> {
    resource_id.id.trim().is_empty().then(|| {
        let mut diagnostic = Diagnostic::new(
            DIAG_INVALID_RESOURCE_ID,
            DiagnosticSeverity::Error,
            "renderer.contract",
            format!("{label} id must not be empty or whitespace"),
        );
        diagnostic.asset = Some(resource_id.clone());
        diagnostic
    })
}

fn validate_mesh_upload(upload: &MeshUpload) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = validate_resource_id(&upload.mesh_id, "mesh") {
        diagnostics.push(diagnostic);
    }

    let stride = upload.vertex_format.stride_bytes() as usize;
    let expected_vertex_bytes = (upload.vertex_count as usize).checked_mul(stride);
    if upload.vertex_count == 0
        || expected_vertex_bytes.is_none()
        || expected_vertex_bytes != Some(upload.vertex_bytes.len())
    {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_MESH_VERTICES,
            DiagnosticSeverity::Error,
            "renderer.contract",
            format!(
                "mesh vertex data must contain exactly vertex_count ({}) * stride ({stride}) bytes; got {}",
                upload.vertex_count,
                upload.vertex_bytes.len()
            ),
        ));
    }

    let index_size = upload.index_format.byte_size();
    let expected_index_bytes = (upload.index_count as usize).checked_mul(index_size);
    if upload.index_count == 0
        || !upload.index_count.is_multiple_of(3)
        || expected_index_bytes.is_none()
        || expected_index_bytes != Some(upload.index_bytes.len())
    {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_MESH_INDICES,
            DiagnosticSeverity::Error,
            "renderer.contract",
            format!(
                "triangle-list index data must contain a non-zero multiple of three indices and exactly index_count ({}) * index size ({index_size}) bytes; got {}",
                upload.index_count,
                upload.index_bytes.len()
            ),
        ));
    }

    let bounds_finite = upload
        .bounds
        .min
        .iter()
        .chain(upload.bounds.max.iter())
        .all(|value| value.is_finite());
    let bounds_ordered = (0..3).all(|axis| upload.bounds.min[axis] <= upload.bounds.max[axis]);
    if !bounds_finite || !bounds_ordered {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_MESH_BOUNDS,
            DiagnosticSeverity::Error,
            "renderer.contract",
            "mesh bounds must be finite and min must not exceed max",
        ));
    }

    diagnostics
}

fn validate_texture_upload(upload: &TextureUpload) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = validate_resource_id(&upload.texture_id, "texture") {
        diagnostics.push(diagnostic);
    }
    if upload.width == 0 || upload.height == 0 {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_TEXTURE_DIMENSIONS,
            DiagnosticSeverity::Error,
            "renderer.contract",
            format!(
                "texture dimensions must be non-zero, got {}x{}",
                upload.width, upload.height
            ),
        ));
        return diagnostics;
    }

    if upload.mip_levels.is_empty() {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_TEXTURE_MIPS,
            DiagnosticSeverity::Error,
            "renderer.contract",
            "texture must contain at least mip level zero",
        ));
        return diagnostics;
    }

    let maximum_mips = u32::BITS - upload.width.max(upload.height).leading_zeros();
    if upload.mip_levels.len() > maximum_mips as usize {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_TEXTURE_MIPS,
            DiagnosticSeverity::Error,
            "renderer.contract",
            format!(
                "texture {}x{} can contain at most {maximum_mips} mip levels; got {}",
                upload.width,
                upload.height,
                upload.mip_levels.len()
            ),
        ));
    }

    let bytes_per_pixel = upload.format.bytes_per_pixel();
    let mut expected_width = upload.width;
    let mut expected_height = upload.height;
    for (level_index, level) in upload.mip_levels.iter().enumerate() {
        if level.width != expected_width || level.height != expected_height {
            diagnostics.push(Diagnostic::new(
                DIAG_INVALID_TEXTURE_MIPS,
                DiagnosticSeverity::Error,
                "renderer.contract",
                format!(
                    "mip {level_index} must be {expected_width}x{expected_height}, got {}x{}",
                    level.width, level.height
                ),
            ));
        }
        let expected_bytes = (expected_width as usize)
            .checked_mul(expected_height as usize)
            .and_then(|pixels| pixels.checked_mul(bytes_per_pixel));
        if expected_bytes.is_none() || expected_bytes != Some(level.bytes.len()) {
            diagnostics.push(Diagnostic::new(
                DIAG_INVALID_TEXTURE_MIPS,
                DiagnosticSeverity::Error,
                "renderer.contract",
                format!(
                    "mip {level_index} must contain exactly {expected_width} * {expected_height} * {bytes_per_pixel} RGBA bytes; got {}",
                    level.bytes.len()
                ),
            ));
        }
        expected_width = (expected_width / 2).max(1);
        expected_height = (expected_height / 2).max(1);
    }

    diagnostics
}

fn validate_material_upload(upload: &MaterialUpload) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if let Some(diagnostic) = validate_resource_id(&upload.material_id, "material") {
        diagnostics.push(diagnostic);
    }
    for (texture_id, label) in upload.texture_references().into_iter().zip([
        "base-color texture",
        "normal texture",
        "metallic-roughness texture",
        "occlusion texture",
        "emissive texture",
    ]) {
        if let Some(texture_id) = texture_id {
            if let Some(diagnostic) = validate_resource_id(texture_id, label) {
                diagnostics.push(diagnostic);
            }
        }
    }

    let factors_valid = upload
        .base_color
        .iter()
        .chain(upload.emissive.iter())
        .chain([
            &upload.metallic,
            &upload.roughness,
            &upload.ambient_occlusion,
        ])
        .all(|value| value.is_finite() && (0.0..=1.0).contains(value));
    if !factors_valid {
        diagnostics.push(Diagnostic::new(
            DIAG_INVALID_MATERIAL_VALUES,
            DiagnosticSeverity::Error,
            "renderer.contract",
            "material color, emissive, metallic, roughness, and ambient occlusion must be finite values in [0, 1]",
        ));
    }

    if let Transparency::Masked { cutoff } = &upload.transparency {
        if !cutoff.is_finite() || !(0.0..=1.0).contains(cutoff) {
            diagnostics.push(Diagnostic::new(
                DIAG_INVALID_MATERIAL_VALUES,
                DiagnosticSeverity::Error,
                "renderer.contract",
                "masked material cutoff must be finite and in [0, 1]",
            ));
        }
    }
    diagnostics
}

#[cfg(test)]
mod contract_layout_tests {
    #[test]
    fn pbr32_vertex_has_the_documented_stride() {
        assert_eq!(
            std::mem::size_of::<crate::Pbr32Vertex>(),
            crate::PBR32_VERTEX_STRIDE as usize
        );
        assert_eq!(
            crate::MeshVertexFormat::Skinned64.stride_bytes(),
            crate::SKINNED64_VERTEX_STRIDE
        );
        assert_eq!(crate::SKINNED64_VERTEX_STRIDE, 64);
    }
}
