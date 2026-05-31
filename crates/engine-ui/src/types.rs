use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::color::Color;
use crate::layout::Layout;

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

impl ElementId {
    /// An invalid / sentinel element ID.
    pub const INVALID: Self = Self(u32::MAX);
}

// ---------------------------------------------------------------------------
// Core data types
// ---------------------------------------------------------------------------

/// Axis-aligned rectangle defined by position and size.
///
/// Coordinate system: +X right, +Y down, origin at top-left.
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

    /// Returns `true` if the point `(px, py)` lies inside this rect.
    ///
    /// A zero-sized rect never contains any point.
    #[inline]
    pub fn contains(&self, px: f32, py: f32) -> bool {
        self.width > 0.0
            && self.height > 0.0
            && px >= self.x
            && px <= self.x + self.width
            && py >= self.y
            && py <= self.y + self.height
    }
}

// ---------------------------------------------------------------------------
// UiElementKind
// ---------------------------------------------------------------------------

/// The visual kind of a [`UiElement`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum UiElementKind {
    /// A solid-colour filled rectangle.
    Panel { color: Color },
    /// A texture-backed image.
    Image { texture_id: String, color: Color },
    /// Text content (rendered as a placeholder quad without actual font
    /// rasterisation).
    Text {
        content: String,
        font_size: f32,
        color: Color,
    },
    /// A clickable button.
    Button {
        label: String,
        normal_color: Color,
        hover_color: Color,
        pressed_color: Color,
        /// Optional string identifier for callback dispatch.
        callback_id: Option<String>,
    },
    /// An on/off toggle switch.
    Toggle {
        label: String,
        /// Whether the toggle is currently in the "on" position.
        is_on: bool,
        /// Colour when toggled on.
        color_on: Color,
        /// Colour when toggled off.
        color_off: Color,
        /// Optional callback identifier for state-change events.
        callback_id: Option<String>,
    },
    /// A checkbox with a label.
    Checkbox {
        label: String,
        /// Whether the checkbox is checked.
        checked: bool,
        /// Colour of the check mark / box.
        color: Color,
        /// Optional callback identifier for state-change events.
        callback_id: Option<String>,
    },
    /// A draggable slider for picking a float value.
    Slider {
        label: String,
        /// Current value.
        value: f32,
        /// Minimum value.
        min: f32,
        /// Maximum value.
        max: f32,
        /// Optional callback identifier for value-change events.
        callback_id: Option<String>,
    },
    /// A scrollable container for child elements.
    ScrollView {
        /// Current horizontal scroll offset.
        scroll_x: f32,
        /// Current vertical scroll offset.
        scroll_y: f32,
        /// Logical content width (may exceed the element's rect width).
        content_width: f32,
        /// Logical content height (may exceed the element's rect height).
        content_height: f32,
        /// Background colour of the scroll viewport.
        color: Color,
    },
}

// ---------------------------------------------------------------------------
// UiElement
// ---------------------------------------------------------------------------

/// A single UI element within a [`Canvas`](crate::Canvas).
///
/// Each element has an anchor-based [`Layout`] that is resolved to a pixel
/// rectangle by [`Canvas::layout_all`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UiElement {
    /// Unique identifier within the canvas.
    pub id: ElementId,
    /// The visual appearance / behaviour of this element.
    pub kind: UiElementKind,
    /// Anchor-based layout descriptor (persistent, user-set).
    pub layout: Layout,
    /// Render order — higher values are drawn on top of lower values.
    pub z_order: i32,
    /// Whether the element is interactive and visible.
    /// Disabled elements are skipped during rendering and input.
    pub enabled: bool,
    /// IDs of child elements (relative to this element's rect).
    pub children: Vec<ElementId>,
    /// **Computed** pixel rect — populated by [`Canvas::layout_all`].
    /// Read by the batch builder and hit-tester.
    pub rect: UiRect,
}

impl UiElement {
    /// Create a new element with the given kind and layout.
    ///
    /// The remaining fields are set to sensible defaults:
    /// - `id`: [`ElementId::INVALID`] (overwritten by [`Canvas::add_element`])
    /// - `z_order`: 0
    /// - `enabled`: true
    /// - `children`: empty
    /// - `rect`: [`UiRect::ZERO`]
    pub fn new(kind: UiElementKind, layout: Layout) -> Self {
        Self {
            id: ElementId::INVALID,
            kind,
            layout,
            z_order: 0,
            enabled: true,
            children: Vec::new(),
            rect: UiRect::ZERO,
        }
    }

    /// Builder-style: set the z-order.
    pub fn with_z_order(mut self, z_order: i32) -> Self {
        self.z_order = z_order;
        self
    }

    /// Builder-style: set the enabled flag.
    pub fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Builder-style: set child element IDs.
    pub fn with_children(mut self, children: Vec<ElementId>) -> Self {
        self.children = children;
        self
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;

    #[test]
    fn element_id_ordering() {
        let a = ElementId(1);
        let b = ElementId(2);
        assert!(a < b);
        assert_eq!(a.max(b), b);
    }

    #[test]
    fn element_id_invalid_sentinel() {
        assert_eq!(ElementId::INVALID, ElementId(u32::MAX));
    }

    #[test]
    fn uirect_contains() {
        let r = UiRect::new(10.0, 20.0, 100.0, 50.0);
        assert!(r.contains(10.0, 20.0)); // top-left
        assert!(r.contains(110.0, 70.0)); // bottom-right
        assert!(!r.contains(9.0, 20.0)); // left of rect
        assert!(!r.contains(10.0, 19.0)); // above rect
        assert!(!r.contains(111.0, 70.0)); // right of rect
        assert!(!r.contains(10.0, 71.0)); // below rect
    }

    #[test]
    fn uirect_zero() {
        let r = UiRect::ZERO;
        assert_eq!(r.x, 0.0);
        assert_eq!(r.y, 0.0);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
        assert!(!r.contains(0.0, 0.0)); // zero-size contains nothing
    }

    #[test]
    fn uielement_new_defaults() {
        let kind = UiElementKind::Panel {
            color: Color::WHITE,
        };
        let layout = Layout::FILL;
        let el = UiElement::new(kind.clone(), layout);
        assert_eq!(el.id, ElementId::INVALID);
        assert_eq!(el.kind, kind);
        assert_eq!(el.layout, Layout::FILL);
        assert_eq!(el.z_order, 0);
        assert!(el.enabled);
        assert!(el.children.is_empty());
        assert_eq!(el.rect, UiRect::ZERO);
    }

    #[test]
    fn uielement_builder_methods() {
        let el = UiElement::new(
            UiElementKind::Text {
                content: "hi".into(),
                font_size: 16.0,
                color: Color::WHITE,
            },
            Layout::FILL,
        )
        .with_z_order(5)
        .with_enabled(false)
        .with_children(vec![ElementId(1), ElementId(2)]);

        assert_eq!(el.z_order, 5);
        assert!(!el.enabled);
        assert_eq!(el.children.len(), 2);
    }
}
