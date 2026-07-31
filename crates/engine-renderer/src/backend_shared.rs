//! Backend-neutral CPU preparation shared by concrete scene renderers.
//!
//! This module deliberately stops at portable frame planning and byte packing.
//! GPU resource ownership, descriptor binding, pipeline selection, and command
//! recording remain responsibilities of each backend.

mod environment;
mod frame;
mod ordering;
mod post_process;
mod ui;

pub use environment::*;
pub use frame::*;
pub use ordering::*;
pub use post_process::*;
pub use ui::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        AssetId, ClearFlags, ContractVersion, EnvironmentSettings, PassGraphConfig,
        PassGraphOutputMode, PostProcessSettings, Rect, ReflectionProbe, RenderFrameInput,
        RenderOptions, RenderView, ToneMapping, RENDERER_INPUT_CONTRACT,
    };
    use glam::Vec3;

    fn view() -> RenderView {
        RenderView {
            view_id: 1,
            camera_entity: None,
            viewport: Rect::FULL,
            viewport_rect_normalized: Rect::FULL,
            view_matrix: crate::IDENTITY_MAT4,
            projection_matrix: crate::IDENTITY_MAT4,
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: crate::ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0, 0.0, 0.0, 1.0],
            },
            stack_order: 0,
            frustum: None,
        }
    }

    fn frame() -> RenderFrameInput {
        RenderFrameInput {
            contract_version: ContractVersion::from(RENDERER_INPUT_CONTRACT),
            frame_index: 0,
            views: vec![view()],
            drawables: Vec::new(),
            skinned_items: Vec::new(),
            particle_batches: Vec::new(),
            materials: Vec::new(),
            meshes: Vec::new(),
            lights: Vec::new(),
            debug_primitives: Vec::new(),
            ui_batches: Vec::new(),
            render_options: RenderOptions::default(),
            stats_scope: None,
            extraction_stats: None,
        }
    }

    #[test]
    fn viewport_maps_fractional_normalized_edges_conservatively() {
        let viewport = prepare_normalized_viewport(
            Rect {
                min: [0.25, 0.1],
                max: [0.75, 0.9],
            },
            1600,
            900,
        )
        .unwrap();
        assert_eq!([viewport.x, viewport.y], [400.0, 90.0]);
        assert_eq!([viewport.width, viewport.height], [800.0, 720.0]);
        assert_eq!(
            viewport.scissor,
            PixelRect {
                x: 400,
                y: 90,
                width: 800,
                height: 720
            }
        );
    }

    #[test]
    fn frame_contract_is_parameterized_by_backend_capabilities() {
        const HDR: &[PassGraphOutputMode] = &[PassGraphOutputMode::HdrThenToneMap];
        const ONE_X: &[u8] = &[1];
        const CLEAR: &[ClearFlags] = &[ClearFlags::ColorAndDepth, ClearFlags::Skybox];
        let capabilities = BackendFrameCapabilities {
            allowed_output_modes: HDR,
            allowed_msaa_samples: ONE_X,
            require_view: true,
            require_matching_view_msaa: true,
            require_matching_viewports: true,
            allowed_clear_flags: CLEAR,
        };
        let mut input = frame();
        assert_eq!(
            validate_backend_frame_contract(&input, capabilities),
            Ok(())
        );
        input.render_options.msaa_samples = 4;
        assert_eq!(
            validate_backend_frame_contract(&input, capabilities),
            Err(FrameContractViolation::UnsupportedMsaa)
        );
        input.render_options.msaa_samples = 1;
        input.render_options.pass_graph_config = PassGraphConfig {
            output_mode: PassGraphOutputMode::DirectToSwapchain,
            ..PassGraphConfig::default()
        };
        assert_eq!(
            validate_backend_frame_contract(&input, capabilities),
            Err(FrameContractViolation::UnsupportedOutputMode)
        );
    }

    #[test]
    fn environment_selection_prefers_priority_then_distance() {
        let global = AssetId::new("global");
        let nearby = AssetId::new("nearby");
        let priority = AssetId::new("priority");
        let settings = EnvironmentSettings {
            environment_map: Some(global.clone()),
            reflection_probes: vec![
                ReflectionProbe {
                    entity: None,
                    environment_map: nearby,
                    position: [0.0; 3],
                    half_extents: [10.0; 3],
                    blend_distance: 0.0,
                    priority: 0,
                },
                ReflectionProbe {
                    entity: None,
                    environment_map: priority.clone(),
                    position: [5.0, 0.0, 0.0],
                    half_extents: [10.0; 3],
                    blend_distance: 0.0,
                    priority: 1,
                },
            ],
            ..EnvironmentSettings::default()
        };
        assert_eq!(
            select_environment_map(&settings, Vec3::ZERO),
            Some(&priority)
        );
        assert_eq!(
            select_environment_map(&settings, Vec3::splat(100.0)),
            Some(&global)
        );
    }

    #[test]
    fn tone_map_plan_is_one_canonical_backend_contract() {
        let mut post = PostProcessSettings::default();
        post.bloom.enabled = true;
        post.color_grading.enabled = true;
        post.vignette.enabled = true;
        post.planetary_lens.enabled = true;
        let plan = prepare_tone_map_plan(
            ToneMapping::Reinhard,
            Some(ARTISTIC_LIGHTING_REFERENCE_EV100 + 2.0),
            post,
            ToneMapPlanOptions {
                output_is_srgb: true,
                weighted_oit_resolve: true,
            },
        )
        .unwrap();
        assert_eq!(plan.mode, TONE_MAP_MODE_REINHARD);
        assert_eq!(plan.exposure, 0.25);
        assert_eq!(plan.output_is_srgb, 1);
        assert_eq!(plan.effect_flags, 0b1_1111);
        assert_eq!(plan.contrast[1], post.planetary_lens.barrel_distortion);
        assert_eq!(plan.vignette[3], post.planetary_lens.chromatic_aberration);
        assert_eq!(plan.to_bytes().len(), ToneMapPlan::SIZE);
    }
}
