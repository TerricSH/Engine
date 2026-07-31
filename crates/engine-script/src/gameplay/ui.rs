use serde::{Deserialize, Serialize};

use super::validation::validate_entity_id;

/// Resulting value carried by a stateful runtime-UI event.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum GameplayUiValue {
    Bool(bool),
    Float(f32),
}

/// One gameplay-facing click emitted by the runtime UI for the current frame.
///
/// The event identifies both the source canvas and element even when no
/// callback id was authored. Stateful controls also report their retained
/// value after the interaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayUiEvent {
    pub canvas_id: String,
    pub element_id: u32,
    pub callback_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<GameplayUiValue>,
}

/// RGBA colour used by managed runtime-UI commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GameplayUiColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

/// Anchor-and-offset layout sent by a managed gameplay script.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct GameplayUiLayout {
    pub anchor_min: [f32; 2],
    pub anchor_max: [f32; 2],
    pub offset_min: [f32; 2],
    pub offset_max: [f32; 2],
}

/// Viewport scaling policy selected by managed Canvas code.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameplayUiScaleMode {
    #[default]
    Fixed,
    FitWidth,
    FitHeight,
}

/// One retained UI element authored through the managed class API.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum GameplayUiElement {
    Panel {
        layout: GameplayUiLayout,
        color: GameplayUiColor,
        z_order: i32,
    },
    Image {
        layout: GameplayUiLayout,
        texture_id: String,
        color: GameplayUiColor,
        z_order: i32,
    },
    Text {
        layout: GameplayUiLayout,
        text: String,
        font_size: f32,
        color: GameplayUiColor,
        z_order: i32,
    },
    Button {
        layout: GameplayUiLayout,
        label: String,
        normal_color: GameplayUiColor,
        hover_color: GameplayUiColor,
        pressed_color: GameplayUiColor,
        callback_id: Option<String>,
        z_order: i32,
    },
    Toggle {
        layout: GameplayUiLayout,
        label: String,
        is_on: bool,
        color_on: GameplayUiColor,
        color_off: GameplayUiColor,
        callback_id: Option<String>,
        z_order: i32,
    },
    Checkbox {
        layout: GameplayUiLayout,
        label: String,
        checked: bool,
        color: GameplayUiColor,
        callback_id: Option<String>,
        z_order: i32,
    },
    Slider {
        layout: GameplayUiLayout,
        label: String,
        value: f32,
        min: f32,
        max: f32,
        callback_id: Option<String>,
        z_order: i32,
    },
    ScrollView {
        layout: GameplayUiLayout,
        content_width: f32,
        content_height: f32,
        color: GameplayUiColor,
        z_order: i32,
    },
}

/// Deferred UI mutations emitted by the managed `UICanvas`/`UIElement`
/// classes. The engine validates and applies these commands at the same frame
/// boundary as scene mutations, so process-host scripts never receive native
/// pointers or ECS handles.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GameplayUiCommand {
    CreateCanvas {
        canvas_id: String,
        width: f32,
        height: f32,
    },
    RemoveCanvas {
        canvas_id: String,
    },
    ResizeCanvas {
        canvas_id: String,
        width: f32,
        height: f32,
    },
    SetCanvasScaleMode {
        canvas_id: String,
        scale_mode: GameplayUiScaleMode,
    },
    ClearCanvas {
        canvas_id: String,
    },
    AddElement {
        canvas_id: String,
        element_id: u32,
        element: GameplayUiElement,
    },
    RemoveElement {
        canvas_id: String,
        element_id: u32,
    },
    SetElementEnabled {
        canvas_id: String,
        element_id: u32,
        enabled: bool,
    },
    SetText {
        canvas_id: String,
        element_id: u32,
        text: String,
    },
    SetToggleValue {
        canvas_id: String,
        element_id: u32,
        is_on: bool,
    },
    SetCheckboxValue {
        canvas_id: String,
        element_id: u32,
        checked: bool,
    },
    SetSliderValue {
        canvas_id: String,
        element_id: u32,
        value: f32,
    },
}

impl GameplayUiCommand {
    /// Validate UI data received from an untrusted process-host script.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            Self::CreateCanvas {
                canvas_id,
                width,
                height,
            }
            | Self::ResizeCanvas {
                canvas_id,
                width,
                height,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_canvas_size(*width, *height)
            }
            Self::RemoveCanvas { canvas_id } | Self::ClearCanvas { canvas_id } => {
                validate_canvas_id(canvas_id)
            }
            Self::SetCanvasScaleMode { canvas_id, .. } => validate_canvas_id(canvas_id),
            Self::AddElement {
                canvas_id,
                element_id,
                element,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)?;
                element.validate()
            }
            Self::RemoveElement {
                canvas_id,
                element_id,
            }
            | Self::SetElementEnabled {
                canvas_id,
                element_id,
                ..
            }
            | Self::SetToggleValue {
                canvas_id,
                element_id,
                ..
            }
            | Self::SetCheckboxValue {
                canvas_id,
                element_id,
                ..
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)
            }
            Self::SetText {
                canvas_id,
                element_id,
                text,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)?;
                validate_ui_text(text, "text")
            }
            Self::SetSliderValue {
                canvas_id,
                element_id,
                value,
            } => {
                validate_canvas_id(canvas_id)?;
                validate_ui_element_id(*element_id)?;
                if value.is_finite() {
                    Ok(())
                } else {
                    Err("UI slider value must be finite".into())
                }
            }
        }
    }
}

impl GameplayUiElement {
    fn validate(&self) -> Result<(), String> {
        let layout = match self {
            Self::Panel { layout, .. }
            | Self::Image { layout, .. }
            | Self::Text { layout, .. }
            | Self::Button { layout, .. }
            | Self::Toggle { layout, .. }
            | Self::Checkbox { layout, .. }
            | Self::Slider { layout, .. }
            | Self::ScrollView { layout, .. } => layout,
        };
        validate_ui_layout(layout)?;

        match self {
            Self::Panel { .. } => Ok(()),
            Self::Image { texture_id, .. } => validate_ui_asset_id(texture_id),
            Self::Text {
                text, font_size, ..
            } => {
                validate_ui_text(text, "text")?;
                if font_size.is_finite() && *font_size > 0.0 && *font_size <= 512.0 {
                    Ok(())
                } else {
                    Err("UI font_size must be finite and in the range (0, 512]".into())
                }
            }
            Self::Button {
                label, callback_id, ..
            }
            | Self::Toggle {
                label, callback_id, ..
            }
            | Self::Checkbox {
                label, callback_id, ..
            } => {
                validate_ui_text(label, "label")?;
                validate_ui_callback_id(callback_id.as_deref())
            }
            Self::Slider {
                label,
                value,
                min,
                max,
                callback_id,
                ..
            } => {
                validate_ui_text(label, "label")?;
                validate_ui_callback_id(callback_id.as_deref())?;
                if !value.is_finite() || !min.is_finite() || !max.is_finite() || min > max {
                    return Err(
                        "UI slider value/min/max must be finite and min must not exceed max".into(),
                    );
                }
                if *value < *min || *value > *max {
                    return Err("UI slider value must be between min and max".into());
                }
                Ok(())
            }
            Self::ScrollView {
                content_width,
                content_height,
                ..
            } => {
                if content_width.is_finite()
                    && content_height.is_finite()
                    && *content_width >= 0.0
                    && *content_height >= 0.0
                {
                    Ok(())
                } else {
                    Err("UI scroll-view content dimensions must be finite and non-negative".into())
                }
            }
        }
    }
}

fn validate_canvas_id(canvas_id: &str) -> Result<(), String> {
    validate_entity_id(canvas_id).map_err(|reason| format!("invalid canvas id: {reason}"))
}

fn validate_canvas_size(width: f32, height: f32) -> Result<(), String> {
    if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
        Ok(())
    } else {
        Err("UI canvas width and height must be finite and greater than zero".into())
    }
}

fn validate_ui_element_id(element_id: u32) -> Result<(), String> {
    if element_id > 0 && element_id != u32::MAX {
        Ok(())
    } else {
        Err("UI element_id must be between 1 and 4294967294".into())
    }
}

fn validate_ui_layout(layout: &GameplayUiLayout) -> Result<(), String> {
    if !layout
        .anchor_min
        .iter()
        .chain(layout.anchor_max.iter())
        .chain(layout.offset_min.iter())
        .chain(layout.offset_max.iter())
        .all(|value| value.is_finite())
    {
        return Err("UI layout anchors and offsets must contain only finite values".into());
    }
    if layout
        .anchor_min
        .iter()
        .chain(layout.anchor_max.iter())
        .any(|value| !(0.0..=1.0).contains(value))
    {
        return Err("UI layout anchors must be in the range [0, 1]".into());
    }
    Ok(())
}

fn validate_ui_text(value: &str, field: &str) -> Result<(), String> {
    if value.len() <= 16_384
        && !value
            .chars()
            .any(|character| character.is_control() && character != '\n' && character != '\t')
    {
        Ok(())
    } else {
        Err(format!(
            "UI {field} must contain at most 16384 bytes and no control characters"
        ))
    }
}

fn validate_ui_asset_id(asset_id: &str) -> Result<(), String> {
    if !asset_id.is_empty() && asset_id.len() <= 256 && !asset_id.chars().any(char::is_control) {
        Ok(())
    } else {
        Err("UI texture_id must contain 1 to 256 bytes and no control characters".into())
    }
}

fn validate_ui_callback_id(callback_id: Option<&str>) -> Result<(), String> {
    let Some(callback_id) = callback_id else {
        return Ok(());
    };
    if !callback_id.is_empty()
        && callback_id.len() <= 128
        && !callback_id.chars().any(char::is_control)
    {
        Ok(())
    } else {
        Err("UI callback_id must contain 1 to 128 bytes and no control characters".into())
    }
}
