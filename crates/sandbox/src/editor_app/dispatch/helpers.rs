//! Serialization, validation, input conversion, and error helpers.

use super::*;

pub(super) fn request_requires_revision(request: &EditorRequest) -> bool {
    !matches!(
        request,
        EditorRequest::Ready(_)
            | EditorRequest::GetSnapshot
            | EditorRequest::SetViewportBounds(_)
            | EditorRequest::ViewportInput(_)
            | EditorRequest::PersistLayout(_)
    )
}

pub(super) fn serialize_message(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).unwrap_or_else(|error| {
        format!(
            "{{\"error\":{{\"code\":\"internal\",\"message\":{}}}}}",
            serde_json::to_string(&error.to_string())
                .unwrap_or_else(|_| "\"serialization failed\"".into())
        )
    })
}

pub(super) fn internal_error(error: serde_json::Error) -> BridgeError {
    BridgeError::new(EditorErrorCode::Internal, error.to_string())
}

pub(super) fn validation_error(error: impl Into<String>) -> BridgeError {
    BridgeError::new(EditorErrorCode::ValidationFailed, error)
}

pub(super) fn validate_react_layout(serialized: &str) -> Result<(), BridgeError> {
    const MAX_LAYOUT_BYTES: usize = 128 * 1024;
    const ZONES: [&str; 4] = ["left", "center", "right", "bottom"];
    const PANELS: [&str; 12] = [
        "hierarchy",
        "scene",
        "game",
        "inspector",
        "project",
        "console",
        "material",
        "animation",
        "profiler",
        "terrain",
        "build",
        "settings",
    ];
    if serialized.len() > MAX_LAYOUT_BYTES {
        return Err(validation_error("React layout exceeds 128 KiB"));
    }
    let layout: JsonValue = serde_json::from_str(serialized)
        .map_err(|error| validation_error(format!("React layout is not valid JSON: {error}")))?;
    let root = layout
        .as_object()
        .ok_or_else(|| validation_error("React layout must be a JSON object"))?;
    let zones = root
        .get("zones")
        .and_then(JsonValue::as_object)
        .ok_or_else(|| validation_error("React layout must define dock zones"))?;
    for zone_id in ZONES {
        let zone = zones
            .get(zone_id)
            .and_then(JsonValue::as_object)
            .ok_or_else(|| validation_error(format!("React layout is missing '{zone_id}'")))?;
        let panels = zone
            .get("panels")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| {
                validation_error(format!("React layout zone '{zone_id}' has no panel list"))
            })?;
        if !panels
            .iter()
            .all(|panel| panel.as_str().is_some_and(|panel| PANELS.contains(&panel)))
        {
            return Err(validation_error(format!(
                "React layout zone '{zone_id}' contains an unknown panel"
            )));
        }
        let active = zone
            .get("active")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| {
                validation_error(format!("React layout zone '{zone_id}' has no active panel"))
            })?;
        if !PANELS.contains(&active) {
            return Err(validation_error(format!(
                "React layout zone '{zone_id}' has an unknown active panel"
            )));
        }
        if zone.get("collapsed").and_then(JsonValue::as_bool).is_none() {
            return Err(validation_error(format!(
                "React layout zone '{zone_id}' has no collapsed state"
            )));
        }
    }
    for dimension in ["leftWidth", "rightWidth", "bottomHeight"] {
        if root.get(dimension).and_then(JsonValue::as_f64).is_none() {
            return Err(validation_error(format!(
                "React layout is missing numeric '{dimension}'"
            )));
        }
    }
    Ok(())
}

pub(super) fn camera_params_are_finite(params: &CameraParams) -> bool {
    params.pitch.is_finite()
        && params.yaw.is_finite()
        && params.distance.is_finite()
        && params.target.iter().all(|value| value.is_finite())
        && params.speed.is_finite()
}

pub(super) fn io_error(error: impl Into<String>) -> BridgeError {
    BridgeError::new(EditorErrorCode::IoFailed, error)
}

pub(super) fn runtime_unavailable() -> BridgeError {
    BridgeError::new(
        EditorErrorCode::RuntimeUnavailable,
        "The editor runtime is not initialized",
    )
}

pub(super) fn selection_error() -> BridgeError {
    BridgeError::new(EditorErrorCode::SelectionRequired, "Select an entity first")
}

pub(super) fn job_conflict(message: String) -> BridgeError {
    BridgeError::new(EditorErrorCode::Conflict, message)
}

pub(super) fn not_found(kind: &str, value: &str) -> BridgeError {
    BridgeError::new(
        EditorErrorCode::NotFound,
        format!("{kind} '{value}' was not found"),
    )
}

pub(super) fn allocate_entity_id(app: &EditorApp) -> String {
    let existing = app
        .editor_scene
        .as_ref()
        .map(|scene| scene.scene.entities.as_slice());
    for sequence in 1_u64.. {
        let candidate = format!("entity-{sequence:04}");
        if existing.is_none_or(|entities| {
            entities
                .iter()
                .all(|entity| entity.persistent_id != candidate)
        }) {
            return candidate;
        }
    }
    unreachable!("u64 entity IDs cannot be exhausted")
}

pub(super) fn web_key_code(code: &str) -> Option<platform::KeyCode> {
    use platform::KeyCode;
    Some(match code {
        "Escape" => KeyCode::Escape,
        "Space" => KeyCode::Space,
        "Enter" | "NumpadEnter" => KeyCode::Enter,
        "Backspace" => KeyCode::Backspace,
        "Tab" => KeyCode::Tab,
        "Delete" => KeyCode::Delete,
        "ArrowLeft" => KeyCode::Left,
        "ArrowRight" => KeyCode::Right,
        "ArrowUp" => KeyCode::Up,
        "ArrowDown" => KeyCode::Down,
        "KeyA" => KeyCode::A,
        "KeyB" => KeyCode::B,
        "KeyC" => KeyCode::C,
        "KeyD" => KeyCode::D,
        "KeyE" => KeyCode::E,
        "KeyF" => KeyCode::F,
        "KeyG" => KeyCode::G,
        "KeyH" => KeyCode::H,
        "KeyI" => KeyCode::I,
        "KeyJ" => KeyCode::J,
        "KeyK" => KeyCode::K,
        "KeyL" => KeyCode::L,
        "KeyM" => KeyCode::M,
        "KeyN" => KeyCode::N,
        "KeyO" => KeyCode::O,
        "KeyP" => KeyCode::P,
        "KeyQ" => KeyCode::Q,
        "KeyR" => KeyCode::R,
        "KeyS" => KeyCode::S,
        "KeyT" => KeyCode::T,
        "KeyU" => KeyCode::U,
        "KeyV" => KeyCode::V,
        "KeyW" => KeyCode::W,
        "KeyX" => KeyCode::X,
        "KeyY" => KeyCode::Y,
        "KeyZ" => KeyCode::Z,
        "Digit0" => KeyCode::Key0,
        "Digit1" => KeyCode::Key1,
        "Digit2" => KeyCode::Key2,
        "Digit3" => KeyCode::Key3,
        "Digit4" => KeyCode::Key4,
        "Digit5" => KeyCode::Key5,
        "Digit6" => KeyCode::Key6,
        "Digit7" => KeyCode::Key7,
        "Digit8" => KeyCode::Key8,
        "Digit9" => KeyCode::Key9,
        "F1" => KeyCode::F1,
        "F2" => KeyCode::F2,
        "F3" => KeyCode::F3,
        "F4" => KeyCode::F4,
        "F5" => KeyCode::F5,
        "F6" => KeyCode::F6,
        "F7" => KeyCode::F7,
        "F8" => KeyCode::F8,
        "F9" => KeyCode::F9,
        "F10" => KeyCode::F10,
        "F11" => KeyCode::F11,
        "F12" => KeyCode::F12,
        _ => return None,
    })
}

#[cfg(feature = "target-desktop")]
pub(super) fn web_modifiers(modifiers: InputModifiers) -> platform::Modifiers {
    platform::Modifiers {
        ctrl: modifiers.control,
        shift: modifiers.shift,
        alt: modifiers.alt,
        logo: modifiers.meta,
    }
}

pub(super) fn css_pointer_to_physical(x: f32, y: f32, scale_factor: f64) -> Vec2 {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor as f32
    } else {
        1.0
    };
    Vec2::new(x * scale, y * scale)
}
