use super::*;

impl VulkanDevice {
    /// Derive finite clip distances from a canonical right-handed Vulkan
    /// zero-to-one projection matrix.
    ///
    /// Both perspective and orthographic matrices are supported, including
    /// reversed-Z variants. Infinite projections are rejected because finite
    /// CSM partitions require a real far plane.
    pub(crate) fn derive_rh_zo_clip_planes(
        projection: &glam::Mat4,
    ) -> Result<(f32, f32), CascadeDataError> {
        const MATRIX_EPSILON: f32 = 1.0e-5;
        const DENOMINATOR_EPSILON: f32 = 1.0e-8;

        if !projection
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::NonFiniteProjection);
        }

        // Canonical RH projections make clip.w depend only on view-space z
        // (perspective) or remain one (orthographic). Reject arbitrary/oblique
        // matrices whose clip distances cannot be recovered by these formulas.
        if projection.x_axis.w.abs() > MATRIX_EPSILON || projection.y_axis.w.abs() > MATRIX_EPSILON
        {
            return Err(CascadeDataError::UnsupportedProjection);
        }

        let a = projection.z_axis.z;
        let b = projection.w_axis.z;
        let (depth_at_zero, depth_at_one) = if (projection.z_axis.w + 1.0).abs() <= MATRIX_EPSILON
            && projection.w_axis.w.abs() <= MATRIX_EPSILON
        {
            // Perspective: ndc_z(d) = -a + b / d, where d = -view_z.
            if a.abs() <= DENOMINATOR_EPSILON || (a + 1.0).abs() <= DENOMINATOR_EPSILON {
                return Err(CascadeDataError::InvalidClipPlanes);
            }
            (b / a, b / (a + 1.0))
        } else if projection.z_axis.w.abs() <= MATRIX_EPSILON
            && (projection.w_axis.w - 1.0).abs() <= MATRIX_EPSILON
        {
            // Orthographic: ndc_z(d) = -a*d + b.
            if a.abs() <= DENOMINATOR_EPSILON {
                return Err(CascadeDataError::InvalidClipPlanes);
            }
            (b / a, (b - 1.0) / a)
        } else {
            return Err(CascadeDataError::UnsupportedProjection);
        };

        let near = depth_at_zero.min(depth_at_one);
        let far = depth_at_zero.max(depth_at_one);
        if !near.is_finite()
            || !far.is_finite()
            || near <= DENOMINATOR_EPSILON
            || far <= near + DENOMINATOR_EPSILON
        {
            return Err(CascadeDataError::InvalidClipPlanes);
        }

        Ok((near, far))
    }

    /// Validate and normalize a directional shadow light vector.
    pub(crate) fn normalize_shadow_light_direction(
        direction: glam::Vec3,
    ) -> Result<glam::Vec3, CascadeDataError> {
        let length_squared = direction.length_squared();
        if !direction.is_finite() || !length_squared.is_finite() || length_squared <= 1.0e-12 {
            return Err(CascadeDataError::InvalidLightDirection);
        }
        Ok(direction / length_squared.sqrt())
    }

    /// Compute PSSM cascade split distances in view-space z.
    ///
    /// Returns `[split0, split1, split2]` where `split_i` is the far plane
    /// of cascade `i` (i.e. the distance from the camera in view-space
    /// negative-z direction). Cascade 0 covers `[near..split0]`,
    /// cascade 1 covers `[split0..split1]`, cascade 2 covers `[split1..far]`.
    ///
    /// Uses a practical lambda-blend of logarithmic and uniform partitioning.
    pub(crate) fn compute_cascade_splits(near: f32, far: f32) -> [f32; 3] {
        let lambda = 0.95f32; // bias toward logarithmic
        let mut splits = [0.0f32; 3];
        for (i, split) in splits.iter_mut().enumerate() {
            let t = (i + 1) as f32 / 3.0;
            let log_split = near * (far / near).powf(t);
            let uniform_split = near + (far - near) * t;
            *split = lambda * log_split + (1.0 - lambda) * uniform_split;
        }
        splits
    }

    /// Compute CSM cascade light view-projection matrices.
    ///
    /// Given the camera's view and projection matrices, and the near/far
    /// plane distances, returns:
    /// - `cascade_splits`: `[split0, split1, split2, far]` split distances
    ///   in view-space z
    /// - `light_vps`: 3 light view-projection matrices, one per cascade
    ///
    /// Each cascade's light VP is an orthographic projection that tightly
    /// bounds the corresponding frustum slice when viewed from the (fixed)
    /// light direction.
    pub(crate) fn compute_cascade_data(
        view_matrix: &glam::Mat4,
        proj_matrix: &glam::Mat4,
        near: f32,
        far: f32,
        light_direction: glam::Vec3,
    ) -> Result<([f32; 4], [glam::Mat4; 3]), CascadeDataError> {
        use glam::Vec3;

        if !view_matrix
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::NonFiniteView);
        }
        let view_determinant = view_matrix.determinant();
        if !view_determinant.is_finite() || view_determinant == 0.0 {
            return Err(CascadeDataError::NonInvertibleView);
        }
        if !proj_matrix
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::NonFiniteProjection);
        }
        let projection_determinant = proj_matrix.determinant();
        if !projection_determinant.is_finite() || projection_determinant == 0.0 {
            return Err(CascadeDataError::UnsupportedProjection);
        }
        if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
            return Err(CascadeDataError::InvalidClipPlanes);
        }
        let light_dir = Self::normalize_shadow_light_direction(light_direction)?;

        let splits = Self::compute_cascade_splits(near, far);
        let splits4: [f32; 4] = [splits[0], splits[1], splits[2], far];

        let inv_view = view_matrix.inverse();
        let inv_proj = proj_matrix.inverse();
        if !inv_view
            .to_cols_array()
            .iter()
            .all(|value| value.is_finite())
            || !inv_proj
                .to_cols_array()
                .iter()
                .all(|value| value.is_finite())
        {
            return Err(CascadeDataError::DegenerateFrustum);
        }

        // Unproject both Vulkan depth endpoints. Sorting by positive view-space
        // distance makes the same code work for perspective, orthographic and
        // reversed-Z projections.
        let ndc_xy = [
            glam::vec2(-1.0, -1.0),
            glam::vec2(1.0, -1.0),
            glam::vec2(1.0, 1.0),
            glam::vec2(-1.0, 1.0),
        ];
        let mut frustum_edges = [(Vec3::ZERO, Vec3::ZERO); 4];
        for (index, xy) in ndc_xy.iter().copied().enumerate() {
            let endpoint_zero = inv_proj * glam::vec4(xy.x, xy.y, 0.0, 1.0);
            let endpoint_one = inv_proj * glam::vec4(xy.x, xy.y, 1.0, 1.0);
            if !endpoint_zero.is_finite()
                || !endpoint_one.is_finite()
                || endpoint_zero.w.abs() <= 1.0e-8
                || endpoint_one.w.abs() <= 1.0e-8
            {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            let point_zero = endpoint_zero.truncate() / endpoint_zero.w;
            let point_one = endpoint_one.truncate() / endpoint_one.w;
            let distance_zero = -point_zero.z;
            let distance_one = -point_one.z;
            if !point_zero.is_finite()
                || !point_one.is_finite()
                || !distance_zero.is_finite()
                || !distance_one.is_finite()
                || distance_zero <= 0.0
                || distance_one <= 0.0
            {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            frustum_edges[index] = if distance_zero <= distance_one {
                (point_zero, point_one)
            } else {
                (point_one, point_zero)
            };
        }

        let mut light_vps = [glam::Mat4::IDENTITY; 3];
        let mut prev_split_z = near;

        for cascade in 0..3 {
            let split_z = splits[cascade];
            let near_t = (prev_split_z - near) / (far - near);
            let far_t = (split_z - near) / (far - near);

            // Compute world-space AABB of the cascade frustum slice.
            let mut min_ws = Vec3::splat(f32::MAX);
            let mut max_ws = Vec3::splat(f32::MIN);
            let mut world_corners = [Vec3::ZERO; 8];
            for (edge_index, (near_corner, far_corner)) in frustum_edges.iter().copied().enumerate()
            {
                let slice_near = near_corner.lerp(far_corner, near_t);
                let slice_far = near_corner.lerp(far_corner, far_t);
                let p_near = inv_view * slice_near.extend(1.0);
                let p_far = inv_view * slice_far.extend(1.0);
                if !p_near.is_finite()
                    || !p_far.is_finite()
                    || p_near.w.abs() <= 1.0e-8
                    || p_far.w.abs() <= 1.0e-8
                {
                    return Err(CascadeDataError::DegenerateFrustum);
                }
                let ws_near = p_near.truncate() / p_near.w;
                let ws_far = p_far.truncate() / p_far.w;
                world_corners[edge_index] = ws_near;
                world_corners[edge_index + 4] = ws_far;

                min_ws = min_ws.min(ws_near).min(ws_far);
                max_ws = max_ws.max(ws_near).max(ws_far);
            }

            // Compute light view at the center of the frustum AABB.
            let center = (min_ws + max_ws) * 0.5;
            let radius = (max_ws - min_ws).length() * 0.5;
            if !center.is_finite() || !radius.is_finite() || radius <= 1.0e-6 {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            let light_pos = center - light_dir * (radius + 1.0);
            let up = if light_dir.dot(Vec3::Y).abs() > 0.99 {
                Vec3::Z
            } else {
                Vec3::Y
            };
            let light_view = glam::Mat4::look_at_rh(light_pos, center, up);

            // Compute tight orthographic bounds in light space.
            let mut ls_min = Vec3::splat(f32::MAX);
            let mut ls_max = Vec3::splat(f32::MIN);
            for corner in world_corners {
                let light_space = (light_view * corner.extend(1.0)).truncate();
                if !light_space.is_finite() {
                    return Err(CascadeDataError::DegenerateFrustum);
                }
                ls_min = ls_min.min(light_space);
                ls_max = ls_max.max(light_space);
            }

            // Add a proportional guard band. Light-space Z is negative in
            // front of the RH light camera, hence the sign conversion below.
            let width = ls_max.x - ls_min.x;
            let height = ls_max.y - ls_min.y;
            let depth = ls_max.z - ls_min.z;
            if !width.is_finite()
                || !height.is_finite()
                || !depth.is_finite()
                || width <= 1.0e-6
                || height <= 1.0e-6
                || depth <= 1.0e-6
            {
                return Err(CascadeDataError::DegenerateFrustum);
            }
            let pad_x = (width * 0.025).max(1.0e-3);
            let pad_y = (height * 0.025).max(1.0e-3);
            let pad_z = (depth * 0.025).max(1.0e-3);
            let light_near = (-ls_max.z - pad_z).max(1.0e-4);
            let light_far = (-ls_min.z + pad_z).max(light_near + 1.0e-3);

            let ortho = glam::Mat4::orthographic_rh(
                ls_min.x - pad_x,
                ls_max.x + pad_x,
                ls_min.y - pad_y,
                ls_max.y + pad_y,
                light_near,
                light_far,
            );

            light_vps[cascade] = ortho * light_view;
            prev_split_z = split_z;
        }

        if light_vps.iter().any(|vp| {
            let determinant = vp.determinant();
            !determinant.is_finite()
                || determinant == 0.0
                || !vp.to_cols_array().iter().all(|value| value.is_finite())
        }) {
            return Err(CascadeDataError::DegenerateFrustum);
        }

        Ok((splits4, light_vps))
    }
}
