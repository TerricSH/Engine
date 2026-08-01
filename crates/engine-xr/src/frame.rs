use serde::{Deserialize, Serialize};

use crate::XrError;

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrPose {
    pub orientation: [f32; 4],
    pub position: [f32; 3],
    pub orientation_valid: bool,
    pub position_valid: bool,
    pub tracked: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrFieldOfView {
    pub angle_left: f32,
    pub angle_right: f32,
    pub angle_up: f32,
    pub angle_down: f32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrView {
    pub pose: XrPose,
    pub fov: XrFieldOfView,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrFrameState {
    pub predicted_display_time_nanoseconds: i64,
    pub should_render: bool,
    pub views: [XrView; 2],
    pub head: XrPose,
    pub left_hand: XrPose,
    pub right_hand: XrPose,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum XrEye {
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct XrSwapchainImage {
    pub eye: XrEye,
    pub image_index: u32,
    /// Array layer to render when the native image is layered. Separate-eye
    /// swapchains use layer zero for both images.
    pub array_layer: u32,
    /// Graphics-API-specific image handle, kept opaque at this layer.
    pub native_image: u64,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct XrCameraMatrices {
    pub view: [f32; 16],
    pub projection: [f32; 16],
}

impl XrFrameState {
    /// Convert the predicted OpenXR eye poses and asymmetric fields of view to
    /// the engine's right-handed camera matrices. This is engine-owned math;
    /// gameplay scripts do not need to reconstruct stereo projection rules.
    pub fn stereo_camera_matrices(
        &self,
        near: f32,
        far: f32,
    ) -> Result<[XrCameraMatrices; 2], XrError> {
        if !near.is_finite() || !far.is_finite() || near <= 0.0 || far <= near {
            return Err(XrError::Graphics(
                "XR camera near/far planes must be finite with 0 < near < far".into(),
            ));
        }
        let camera = |index: usize| {
            let eye = self.views[index];
            if !eye.pose.orientation_valid || !eye.pose.position_valid {
                return Err(XrError::Runtime(format!(
                    "XR eye {index} has no valid predicted pose"
                )));
            }
            let orientation = glam::Quat::from_array(eye.pose.orientation);
            if !orientation.is_finite() || orientation.length_squared() <= f32::EPSILON {
                return Err(XrError::Runtime(format!(
                    "XR eye {index} has an invalid orientation"
                )));
            }
            let world = glam::Mat4::from_rotation_translation(
                orientation.normalize(),
                glam::Vec3::from_array(eye.pose.position),
            );
            let projection = asymmetric_projection(eye.fov, near, far)?;
            Ok(XrCameraMatrices {
                view: world.inverse().to_cols_array(),
                projection: projection.to_cols_array(),
            })
        };
        Ok([camera(0)?, camera(1)?])
    }
}

fn asymmetric_projection(fov: XrFieldOfView, near: f32, far: f32) -> Result<glam::Mat4, XrError> {
    let left = fov.angle_left.tan();
    let right = fov.angle_right.tan();
    let down = fov.angle_down.tan();
    let up = fov.angle_up.tan();
    if [left, right, down, up]
        .into_iter()
        .any(|value| !value.is_finite())
        || right <= left
        || up <= down
    {
        return Err(XrError::Runtime(
            "XR view has an invalid field of view".into(),
        ));
    }
    let width = right - left;
    let height = up - down;
    Ok(glam::Mat4::from_cols(
        glam::Vec4::new(2.0 / width, 0.0, 0.0, 0.0),
        glam::Vec4::new(0.0, 2.0 / height, 0.0, 0.0),
        glam::Vec4::new(
            (right + left) / width,
            (up + down) / height,
            far / (near - far),
            -1.0,
        ),
        glam::Vec4::new(0.0, 0.0, (far * near) / (near - far), 0.0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stereo_camera_uses_each_asymmetric_eye_pose() {
        let pose = |x| XrPose {
            orientation: [0.0, 0.0, 0.0, 1.0],
            position: [x, 0.0, 0.0],
            orientation_valid: true,
            position_valid: true,
            tracked: true,
        };
        let fov = XrFieldOfView {
            angle_left: -0.7,
            angle_right: 0.8,
            angle_up: 0.75,
            angle_down: -0.65,
        };
        let frame = XrFrameState {
            views: [
                XrView {
                    pose: pose(-0.03),
                    fov,
                },
                XrView {
                    pose: pose(0.03),
                    fov,
                },
            ],
            ..XrFrameState::default()
        };
        let cameras = frame.stereo_camera_matrices(0.05, 1_000.0).unwrap();
        assert_ne!(cameras[0].view, cameras[1].view);
        assert_eq!(cameras[0].projection, cameras[1].projection);
        assert!(cameras[0]
            .projection
            .into_iter()
            .all(|value| value.is_finite()));
    }
}
