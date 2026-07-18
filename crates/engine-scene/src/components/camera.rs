use crate::Component;
use engine_serialize::Value;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::field_as_f32;

/// Projection type for a camera.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum CameraProjection {
    Perspective,
    Orthographic,
}

/// Camera component per FD-034.
///
/// Provides full camera description including exposure parameters for
/// physically-based rendering.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Camera {
    pub projection: CameraProjection,
    pub near: f32,
    pub far: f32,
    /// Vertical field of view in radians (only for Perspective).
    pub fov_y: f32,
    /// Half-height of the orthographic view volume.
    pub ortho_half_height: f32,
    /// Normalized viewport rectangle `[x, y, w, h]`. `None` means full viewport.
    pub viewport_rect: Option<[f32; 4]>,
    /// Bitmask of render layers this camera renders.
    pub render_layer_mask: u32,
    /// Clear mode bitmask: 1 = color, 2 = depth, 4 = skybox + depth.
    /// Skybox takes precedence when bit 4 is set.
    pub clear_flags: u8,
    /// Clear colour (RGBA).
    pub clear_color: [f32; 4],
    /// Render priority (higher = later).
    pub priority: i32,
    /// MSAA sample count.
    pub msaa_samples: u8,
    /// Whether to use HDR output.
    pub hdr_output: bool,
    /// Aperture (f-stop) for depth-of-field and exposure.
    pub aperture: f32,
    /// Shutter speed in seconds.
    pub shutter_speed: f32,
    /// ISO sensitivity.
    pub iso: f32,
    /// Exposure value compensation.
    pub ev_compensation: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            projection: CameraProjection::Perspective,
            near: 0.1,
            far: 1000.0,
            fov_y: std::f32::consts::FRAC_PI_4,
            ortho_half_height: 5.0,
            viewport_rect: None,
            render_layer_mask: u32::MAX,
            clear_flags: 3,
            clear_color: [0.02, 0.02, 0.06, 1.0],
            priority: 0,
            msaa_samples: 1,
            hdr_output: false,
            aperture: 16.0,
            shutter_speed: 1.0 / 60.0,
            iso: 100.0,
            ev_compensation: 0.0,
        }
    }
}

impl Component for Camera {
    const TYPE_ID: &'static str = "engine.camera";
}

// ---------------------------------------------------------------------------
// Scene field-map serde
// ---------------------------------------------------------------------------

/// Serialize a [`Camera`] into the field-map layout used by scene files.
///
/// This is the single source of truth for the camera field layout: scene
/// loading/saving, the component registry hooks, and the script component
/// bridge all share it.
pub fn serialize_camera_fields(camera: &Camera) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    fields.insert(
        "projection".to_string(),
        Value::Enum(match camera.projection {
            CameraProjection::Perspective => "Perspective".to_string(),
            CameraProjection::Orthographic => "Orthographic".to_string(),
        }),
    );
    fields.insert("near".to_string(), Value::Float32(camera.near));
    fields.insert("far".to_string(), Value::Float32(camera.far));
    fields.insert("fov_y".to_string(), Value::Float32(camera.fov_y));
    fields.insert(
        "ortho_half_height".to_string(),
        Value::Float32(camera.ortho_half_height),
    );
    if let Some(vp) = camera.viewport_rect {
        fields.insert(
            "viewport_rect".to_string(),
            Value::List(vp.iter().map(|v| Value::Float32(*v)).collect()),
        );
    }
    fields.insert(
        "render_layer_mask".to_string(),
        Value::UInt(camera.render_layer_mask as u64),
    );
    fields.insert(
        "clear_flags".to_string(),
        Value::UInt(camera.clear_flags as u64),
    );
    fields.insert("clear_color".to_string(), Value::Color(camera.clear_color));
    fields.insert("priority".to_string(), Value::Int(camera.priority as i64));
    fields.insert(
        "msaa_samples".to_string(),
        Value::UInt(camera.msaa_samples as u64),
    );
    fields.insert("hdr_output".to_string(), Value::Bool(camera.hdr_output));
    fields.insert("aperture".to_string(), Value::Float32(camera.aperture));
    fields.insert(
        "shutter_speed".to_string(),
        Value::Float32(camera.shutter_speed),
    );
    fields.insert("iso".to_string(), Value::Float32(camera.iso));
    fields.insert(
        "ev_compensation".to_string(),
        Value::Float32(camera.ev_compensation),
    );
    fields
}

/// Build a [`Camera`] from a scene field map, applying authored defaults for
/// any missing field with the same tolerance as the scene loader.
pub fn deserialize_camera_fields(fields: &BTreeMap<String, Value>) -> Camera {
    let defaults = Camera::default();
    let projection = match fields.get("projection") {
        Some(Value::Enum(s)) if s == "Orthographic" => CameraProjection::Orthographic,
        _ => CameraProjection::Perspective,
    };
    let near = match fields.get("near") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.near,
    };
    let far = match fields.get("far") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.far,
    };
    let fov_y = match fields.get("fov_y") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.fov_y,
    };
    let ortho_half_height = match fields.get("ortho_half_height") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.ortho_half_height,
    };
    let viewport_rect = match fields.get("viewport_rect") {
        Some(Value::List(items)) if items.len() == 4 => Some([
            field_as_f32(&items[0]),
            field_as_f32(&items[1]),
            field_as_f32(&items[2]),
            field_as_f32(&items[3]),
        ]),
        _ => None,
    };
    let render_layer_mask = match fields.get("render_layer_mask") {
        Some(Value::UInt(v)) => *v as u32,
        _ => defaults.render_layer_mask,
    };
    let clear_flags = match fields.get("clear_flags") {
        Some(Value::UInt(v)) => *v as u8,
        _ => defaults.clear_flags,
    };
    let clear_color = match fields.get("clear_color") {
        Some(Value::Color(c)) => *c,
        _ => defaults.clear_color,
    };
    let priority = match fields.get("priority") {
        Some(Value::Int(v)) => *v as i32,
        _ => defaults.priority,
    };
    let msaa_samples = match fields.get("msaa_samples") {
        Some(Value::UInt(v)) => *v as u8,
        _ => defaults.msaa_samples,
    };
    let hdr_output = match fields.get("hdr_output") {
        Some(Value::Bool(v)) => *v,
        _ => defaults.hdr_output,
    };
    let aperture = match fields.get("aperture") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.aperture,
    };
    let shutter_speed = match fields.get("shutter_speed") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.shutter_speed,
    };
    let iso = match fields.get("iso") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.iso,
    };
    let ev_compensation = match fields.get("ev_compensation") {
        Some(Value::Float32(v)) => *v,
        Some(Value::Float64(v)) => *v as f32,
        _ => defaults.ev_compensation,
    };
    Camera {
        projection,
        near,
        far,
        fov_y,
        ortho_half_height,
        viewport_rect,
        render_layer_mask,
        clear_flags,
        clear_color,
        priority,
        msaa_samples,
        hdr_output,
        aperture,
        shutter_speed,
        iso,
        ev_compensation,
    }
}

/// Registry hook: serialize a type-erased [`Camera`] into its field map.
pub fn serialize_camera(component: &dyn std::any::Any) -> BTreeMap<String, Value> {
    let camera = component.downcast_ref::<Camera>().expect("Camera expected");
    serialize_camera_fields(camera)
}

/// Registry hook: build a type-erased [`Camera`] from a field map.
pub fn deserialize_camera(fields: &BTreeMap<String, Value>) -> Box<dyn std::any::Any> {
    Box::new(deserialize_camera_fields(fields))
}
