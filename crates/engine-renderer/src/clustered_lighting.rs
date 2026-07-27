//! Backend-neutral CPU-built clustered-light lists consumed by forward shaders.
//!
//! Vulkan and DX12 consume this bounded tile/depth partition and byte ABI,
//! keeping light assignment deterministic across rendering backends.

use crate::{LightItem, LightKind, RenderView};
use glam::{Mat4, Vec3};

pub const MAX_LIGHTS: usize = 256;
pub const LIGHT_GPU_SIZE: usize = 64;
pub const LIGHT_HEADER_SIZE: usize = 144;
pub const MAX_CLUSTERS: usize = 32_768;
pub const MAX_CLUSTER_LIGHT_INDICES: usize = 524_288;

const TARGET_TILE_SIZE: f32 = 64.0;
const MAX_TILE_COLUMNS: u32 = 60;
const MAX_TILE_ROWS: u32 = 34;
const DEPTH_SLICE_COUNT: u32 = 16;
const MAX_LIGHTS_PER_CLUSTER: usize = 64;
const DEFAULT_NEAR: f32 = 0.1;
const DEFAULT_FAR: f32 = 1_000.0;

#[derive(Clone, Debug)]
pub struct ClusteredLightFrame {
    pub light_bytes: Vec<u8>,
    pub cluster_grid_bytes: Vec<u8>,
    pub cluster_index_bytes: Vec<u8>,
    pub light_count: u32,
    pub cluster_count: u32,
    pub index_count: u32,
    pub overflowed_assignments: u32,
}

#[derive(Clone, Copy, Debug)]
struct ClusterLayout {
    tile_columns: u32,
    tile_rows: u32,
    depth_slices: u32,
    viewport: [f32; 4],
    near: f32,
    far: f32,
    log_scale: f32,
    log_bias: f32,
}

impl ClusterLayout {
    fn new(view: &RenderView, surface_width: u32, surface_height: u32) -> Self {
        let surface_width = surface_width.max(1) as f32;
        let surface_height = surface_height.max(1) as f32;
        let rect = view.viewport_rect_normalized;
        let viewport_width = (rect.width() * surface_width).max(1.0);
        let viewport_height = (rect.height() * surface_height).max(1.0);
        let tile_columns =
            ((viewport_width / TARGET_TILE_SIZE).ceil() as u32).clamp(1, MAX_TILE_COLUMNS);
        let tile_rows =
            ((viewport_height / TARGET_TILE_SIZE).ceil() as u32).clamp(1, MAX_TILE_ROWS);
        let (near, far) = projection_depth_range(Mat4::from_cols_array(&view.projection_matrix))
            .unwrap_or((DEFAULT_NEAR, DEFAULT_FAR));
        let log_scale = DEPTH_SLICE_COUNT as f32 / (far / near).ln();
        let log_bias = -near.ln() * log_scale;
        Self {
            tile_columns,
            tile_rows,
            depth_slices: DEPTH_SLICE_COUNT,
            viewport: [
                rect.min[0] * surface_width,
                rect.min[1] * surface_height,
                viewport_width,
                viewport_height,
            ],
            near,
            far,
            log_scale,
            log_bias,
        }
    }

    fn cluster_count(self) -> usize {
        self.tile_columns as usize * self.tile_rows as usize * self.depth_slices as usize
    }

    fn depth_slice(self, distance: f32) -> u32 {
        let distance = distance.clamp(self.near, self.far);
        ((distance.ln() * self.log_scale + self.log_bias).floor() as i32)
            .clamp(0, self.depth_slices as i32 - 1) as u32
    }

    fn cluster_index(self, x: u32, y: u32, z: u32) -> usize {
        ((z * self.tile_rows + y) * self.tile_columns + x) as usize
    }
}

/// Build the per-frame clustered-light buffers.
///
/// `lights` excludes the primary directional light stored in the per-frame
/// UBO. Inputs beyond [`MAX_LIGHTS`] are deterministically ignored.
pub fn build_clustered_light_frame(
    lights: &[&LightItem],
    view: &RenderView,
    surface_width: u32,
    surface_height: u32,
) -> ClusteredLightFrame {
    let layout = ClusterLayout::new(view, surface_width, surface_height);
    debug_assert!(layout.cluster_count() <= MAX_CLUSTERS);

    let view_matrix = Mat4::from_cols_array(&view.view_matrix);
    let projection = Mat4::from_cols_array(&view.projection_matrix);
    let camera_position = view_matrix.inverse().w_axis.truncate();
    let accepted_lights = lights.iter().copied().take(MAX_LIGHTS).collect::<Vec<_>>();
    let mut clusters = vec![Vec::<u32>::new(); layout.cluster_count()];
    let mut overflowed_assignments =
        u32::try_from(lights.len().saturating_sub(MAX_LIGHTS)).unwrap_or(u32::MAX);

    for (light_index, light) in accepted_lights.iter().enumerate() {
        let bounds = light_cluster_bounds(light, layout, view_matrix, projection, camera_position);
        let Some((min_x, max_x, min_y, max_y, min_z, max_z)) = bounds else {
            continue;
        };
        for z in min_z..=max_z {
            for y in min_y..=max_y {
                for x in min_x..=max_x {
                    let cluster = &mut clusters[layout.cluster_index(x, y, z)];
                    if cluster.len() < MAX_LIGHTS_PER_CLUSTER {
                        cluster.push(light_index as u32);
                    } else {
                        overflowed_assignments = overflowed_assignments.saturating_add(1);
                    }
                }
            }
        }
    }

    let mut cluster_grid_bytes = Vec::with_capacity(clusters.len() * 8);
    let mut cluster_index_bytes = Vec::new();
    let mut index_count = 0usize;
    for cluster in clusters {
        let offset = index_count;
        let remaining = MAX_CLUSTER_LIGHT_INDICES.saturating_sub(index_count);
        let count = cluster.len().min(remaining);
        for light_index in cluster.into_iter().take(count) {
            cluster_index_bytes.extend_from_slice(&light_index.to_ne_bytes());
        }
        index_count += count;
        cluster_grid_bytes.extend_from_slice(&(offset as u32).to_ne_bytes());
        cluster_grid_bytes.extend_from_slice(&(count as u32).to_ne_bytes());
        if count == 0 && remaining == 0 {
            overflowed_assignments = overflowed_assignments.saturating_add(1);
        }
    }

    let mut light_bytes =
        Vec::with_capacity(LIGHT_HEADER_SIZE + accepted_lights.len() * LIGHT_GPU_SIZE);
    append_uvec4(
        &mut light_bytes,
        [
            accepted_lights.len() as u32,
            layout.tile_columns,
            layout.tile_rows,
            layout.depth_slices,
        ],
    );
    append_uvec4(
        &mut light_bytes,
        [
            layout.cluster_count() as u32,
            index_count as u32,
            MAX_LIGHTS_PER_CLUSTER as u32,
            overflowed_assignments,
        ],
    );
    append_vec4(&mut light_bytes, layout.viewport);
    append_vec4(
        &mut light_bytes,
        [layout.near, layout.far, layout.log_scale, layout.log_bias],
    );
    let inverse_view_projection = (projection * view_matrix).inverse();
    for value in inverse_view_projection.to_cols_array() {
        light_bytes.extend_from_slice(&value.to_ne_bytes());
    }
    append_vec4(
        &mut light_bytes,
        [camera_position.x, camera_position.y, camera_position.z, 1.0],
    );
    for light in &accepted_lights {
        light_bytes.extend_from_slice(&pack_light_gpu_bytes(light));
    }

    ClusteredLightFrame {
        light_count: accepted_lights.len() as u32,
        cluster_count: layout.cluster_count() as u32,
        index_count: index_count as u32,
        overflowed_assignments,
        light_bytes,
        cluster_grid_bytes,
        cluster_index_bytes,
    }
}

fn light_cluster_bounds(
    light: &LightItem,
    layout: ClusterLayout,
    view: Mat4,
    projection: Mat4,
    camera_position: Vec3,
) -> Option<(u32, u32, u32, u32, u32, u32)> {
    if matches!(light.kind, LightKind::Directional) {
        return Some((
            0,
            layout.tile_columns - 1,
            0,
            layout.tile_rows - 1,
            0,
            layout.depth_slices - 1,
        ));
    }

    let position = Vec3::from_array(light.position);
    let range = light.range.max(0.01);
    let camera_distance = position.distance(camera_position);
    if camera_distance - range > layout.far || camera_distance + range < layout.near {
        return None;
    }
    let min_z = layout.depth_slice((camera_distance - range).max(layout.near));
    let max_z = layout.depth_slice((camera_distance + range).min(layout.far));

    let view_position = view * position.extend(1.0);
    let forward_depth = -view_position.z;
    let (min_x, max_x, min_y, max_y) = if forward_depth <= range.max(layout.near) {
        (0, layout.tile_columns - 1, 0, layout.tile_rows - 1)
    } else {
        let clip = projection * view_position;
        if !clip.is_finite() || clip.w <= 0.0 {
            return None;
        }
        let center = clip.truncate() / clip.w;
        let conservative_depth = (forward_depth - range).max(layout.near);
        let radius_x = projection.x_axis.x.abs() * range / conservative_depth;
        let radius_y = projection.y_axis.y.abs() * range / conservative_depth;
        let min_normalized = [
            ((center.x - radius_x) * 0.5 + 0.5).clamp(0.0, 1.0),
            ((center.y - radius_y) * 0.5 + 0.5).clamp(0.0, 1.0),
        ];
        let max_normalized = [
            ((center.x + radius_x) * 0.5 + 0.5).clamp(0.0, 1.0),
            ((center.y + radius_y) * 0.5 + 0.5).clamp(0.0, 1.0),
        ];
        if max_normalized[0] <= 0.0
            || min_normalized[0] >= 1.0
            || max_normalized[1] <= 0.0
            || min_normalized[1] >= 1.0
        {
            return None;
        }
        (
            normalized_tile(min_normalized[0], layout.tile_columns),
            normalized_tile(max_normalized[0], layout.tile_columns),
            normalized_tile(min_normalized[1], layout.tile_rows),
            normalized_tile(max_normalized[1], layout.tile_rows),
        )
    };
    Some((min_x, max_x, min_y, max_y, min_z, max_z))
}

fn normalized_tile(value: f32, tile_count: u32) -> u32 {
    ((value.clamp(0.0, 1.0) * tile_count as f32).floor() as u32).min(tile_count - 1)
}

fn projection_depth_range(projection: Mat4) -> Option<(f32, f32)> {
    let a = projection.z_axis.z;
    let b = projection.w_axis.z;
    let near = b / a;
    let far = b / (a + 1.0);
    (near.is_finite() && far.is_finite() && near > 0.0 && far > near && projection.z_axis.w < 0.0)
        .then_some((near, far))
}

fn pack_light_gpu_bytes(light: &LightItem) -> [u8; LIGHT_GPU_SIZE] {
    let direction = normalize_direction(light.direction);
    let kind = match light.kind {
        LightKind::Directional => 0.0,
        LightKind::Point => 1.0,
        LightKind::Spot => 2.0,
    };
    let range = light.range.max(0.0);
    let quadratic = if range > 0.0 {
        1.0 / (range * range)
    } else {
        0.0
    };
    let spot_cutoff = match (&light.kind, &light.spot_angles) {
        (LightKind::Spot, Some(angles)) => angles.outer.cos(),
        _ => 0.0,
    };
    let values = [
        light.position[0],
        light.position[1],
        light.position[2],
        kind,
        direction[0],
        direction[1],
        direction[2],
        0.0,
        light.color[0],
        light.color[1],
        light.color[2],
        light.intensity,
        range,
        0.0,
        quadratic,
        spot_cutoff,
    ];
    let mut bytes = [0; LIGHT_GPU_SIZE];
    for (index, value) in values.into_iter().enumerate() {
        bytes[index * 4..index * 4 + 4].copy_from_slice(&value.to_ne_bytes());
    }
    bytes
}

pub fn normalize_direction(direction: [f32; 3]) -> [f32; 3] {
    let direction = Vec3::from_array(direction);
    if direction.length_squared() > 0.0 {
        direction.normalize().to_array()
    } else {
        [0.0, -1.0, 0.0]
    }
}

fn append_uvec4(bytes: &mut Vec<u8>, values: [u32; 4]) {
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
}

fn append_vec4(bytes: &mut Vec<u8>, values: [f32; 4]) {
    for value in values {
        bytes.extend_from_slice(&value.to_ne_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ClearFlags, Rect, ShadowMode, SpotAngles, ViewCompose, IDENTITY_MAT4};

    fn view() -> RenderView {
        RenderView {
            view_id: 0,
            camera_entity: None,
            viewport: Rect::FULL,
            viewport_rect_normalized: Rect::FULL,
            view_matrix: IDENTITY_MAT4,
            projection_matrix: Mat4::perspective_rh(60.0_f32.to_radians(), 16.0 / 9.0, 0.1, 100.0)
                .to_cols_array(),
            clear_flags: ClearFlags::ColorAndDepth,
            clear_color: [0.0; 4],
            render_layer_mask: u32::MAX,
            msaa_samples: 1,
            compose: ViewCompose::Base {
                clear: ClearFlags::ColorAndDepth,
                clear_color: [0.0; 4],
            },
            stack_order: 0,
            frustum: None,
        }
    }

    fn point(position: [f32; 3], range: f32) -> LightItem {
        LightItem {
            entity: None,
            kind: LightKind::Point,
            color: [1.0; 3],
            intensity: 2.0,
            range,
            position,
            direction: [0.0, -1.0, 0.0],
            spot_angles: Some(SpotAngles {
                inner: 0.4,
                outer: 0.7,
            }),
            shadow_mode: ShadowMode::Off,
        }
    }

    #[test]
    fn empty_frame_still_writes_a_zero_count_header() {
        let frame = build_clustered_light_frame(&[], &view(), 1920, 1080);
        assert_eq!(frame.light_count, 0);
        assert_eq!(frame.light_bytes.len(), LIGHT_HEADER_SIZE);
        assert_eq!(
            u32::from_ne_bytes(frame.light_bytes[0..4].try_into().unwrap()),
            0
        );
        assert!(frame.cluster_count > 0);
        assert_eq!(frame.index_count, 0);
    }

    #[test]
    fn visible_point_light_populates_only_a_subset_of_clusters() {
        let light = point([0.0, 0.0, -5.0], 1.0);
        let frame = build_clustered_light_frame(&[&light], &view(), 1920, 1080);
        assert_eq!(frame.light_count, 1);
        assert!(frame.index_count > 0);
        assert!(frame.index_count < frame.cluster_count);
        assert_eq!(frame.overflowed_assignments, 0);
    }

    #[test]
    fn light_count_is_bounded_by_the_gpu_contract() {
        let lights = (0..MAX_LIGHTS + 5)
            .map(|index| point([index as f32, 0.0, -5.0], 1.0))
            .collect::<Vec<_>>();
        let references = lights.iter().collect::<Vec<_>>();
        let frame = build_clustered_light_frame(&references, &view(), 1280, 720);
        assert_eq!(frame.light_count, MAX_LIGHTS as u32);
        assert!(frame.overflowed_assignments >= 5);
    }
}
