use engine_serialize::AssetId;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Color;

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Errors that can occur during UI operations.
#[derive(Error, Debug)]
pub enum UiError {
    #[error("element not found: {0:?}")]
    ElementNotFound(ElementId),

    #[error("canvas has no elements")]
    EmptyCanvas,
}

// ---------------------------------------------------------------------------
// ElementId
// ---------------------------------------------------------------------------

/// Unique identifier for a [`UiElement`] within a [`Canvas`](crate::Canvas).
#[derive(
    Clone, Copy, Debug, Default, Eq, Hash, PartialEq, PartialOrd, Ord, Serialize, Deserialize,
)]
pub struct ElementId(pub u32);

// ---------------------------------------------------------------------------
// Core data types
// ---------------------------------------------------------------------------

/// Axis-aligned rectangle defined by position and size.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl UiRect {
    /// Zero-sized rect at the origin.
    pub const ZERO: Self = Self {
        x: 0.0,
        y: 0.0,
        width: 0.0,
        height: 0.0,
    };

    /// Create a new rect.
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }
}

/// A single UI element within a [`Canvas`](crate::Canvas).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiElement {
    pub id: ElementId,
    pub rect: UiRect,
    pub z_order: i32,
    pub visible: bool,
    pub kind: UiElementKind,
}

/// The visual kind of a [`UiElement`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UiElementKind {
    /// A filled rectangle.
    Quad {
        color: Color,
        /// Radius for rounded corners (currently a hint, not implemented).
        corner_radius: f32,
    },
    /// A rectangular outline.
    Border {
        color: Color,
        /// Border thickness in pixels.
        thickness: f32,
    },
    /// Placeholder text rendered as a semi-transparent quad.
    Text {
        content: String,
        font_size: f32,
        color: Color,
    },
    /// Texture-backed image.
    Image { texture: AssetId, tint: Color },
    /// 9-slice scaled texture.
    ///
    /// The `border` field is a [`UiRect`] whose values represent the border
    /// sizes in **both** pixels (destination positioning) and normalized UV
    /// fractions (source texture slicing).  For a texture of size T×T with a
    /// left-border of L pixels, pass `L / T` as `border.x` and let the
    /// destination pixel size equal the same value (or scale proportionally).
    NineSlice {
        texture: AssetId,
        border: UiRect,
        tint: Color,
    },
}
