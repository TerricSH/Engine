#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeModeDto {
    Edit,
    Play,
    Paused,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeModeParams {
    pub mode: RuntimeModeDto,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScreenRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportBoundsParams {
    pub viewport: String,
    pub rect: ScreenRect,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputModifiers {
    #[cfg(feature = "target-desktop")]
    pub alt: bool,
    pub control: bool,
    #[cfg(feature = "target-desktop")]
    pub meta: bool,
    pub shift: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ViewportInput {
    PointerDown {
        pointer_id: i64,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    },
    PointerUp {
        pointer_id: i64,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    },
    PointerMove {
        pointer_id: i64,
        x: f32,
        y: f32,
        button: i16,
        buttons: u16,
        modifiers: InputModifiers,
    },
    PointerCancel {
        pointer_id: i64,
    },
    Wheel {
        x: f32,
        y: f32,
        delta_x: f32,
        delta_y: f32,
        delta_mode: u8,
        modifiers: InputModifiers,
    },
    KeyDown {
        key: String,
        code: String,
        repeat: bool,
        modifiers: InputModifiers,
    },
    KeyUp {
        key: String,
        code: String,
        repeat: bool,
        modifiers: InputModifiers,
    },
    Focus,
    Blur,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewportInputParams {
    pub viewport: String,
    pub event: ViewportInput,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GizmoModeDto {
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GizmoModeParams {
    pub mode: GizmoModeDto,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GizmoSpaceDto {
    Global,
    Local,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GizmoSpaceParams {
    pub mode: GizmoSpaceDto,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetSnappingParams {
    pub enabled: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraParams {
    pub pitch: f32,
    pub yaw: f32,
    pub distance: f32,
    pub target: [f32; 3],
    pub orthographic: bool,
    pub speed: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetGizmosParams {
    pub visible: bool,
}
use super::*;
