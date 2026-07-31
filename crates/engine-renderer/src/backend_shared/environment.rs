use glam::Vec3;

use crate::{AssetId, EnvironmentSettings, ExtractionStats, FrameStats, RenderFrameInput};

/// Pick the highest-priority local reflection probe containing the camera,
/// breaking ties by normalized distance, then fall back to the global map.
pub fn select_environment_map(
    settings: &EnvironmentSettings,
    camera_position: Vec3,
) -> Option<&AssetId> {
    settings
        .reflection_probes
        .iter()
        .filter_map(|probe| {
            let position = Vec3::from_array(probe.position);
            let extents =
                Vec3::from_array(probe.half_extents) + Vec3::splat(probe.blend_distance.max(0.0));
            let offset = (camera_position - position).abs();
            (offset.cmple(extents).all()).then(|| {
                let normalized_distance =
                    (offset / extents.max(Vec3::splat(0.0001))).length_squared();
                (probe, normalized_distance)
            })
        })
        .max_by(|(left, left_distance), (right, right_distance)| {
            left.priority
                .cmp(&right.priority)
                .then_with(|| right_distance.total_cmp(left_distance))
        })
        .map(|(probe, _)| &probe.environment_map)
        .or(settings.environment_map.as_ref())
}

pub fn extraction_stats(input: &RenderFrameInput) -> ExtractionStats {
    input.extraction_stats.unwrap_or(ExtractionStats {
        visible_drawables: u32::try_from(
            input
                .drawables
                .len()
                .saturating_add(input.skinned_items.len()),
        )
        .unwrap_or(u32::MAX),
        culled_drawables: 0,
        visible_lights: u32::try_from(input.lights.len()).unwrap_or(u32::MAX),
        culled_lights: 0,
    })
}

pub fn apply_extraction_stats(stats: &mut FrameStats, input: &RenderFrameInput) {
    let extraction = extraction_stats(input);
    stats.visible_drawables = extraction.visible_drawables;
    stats.culled_drawables = extraction.culled_drawables;
    stats.visible_lights = extraction.visible_lights;
    stats.culled_lights = extraction.culled_lights;
}
