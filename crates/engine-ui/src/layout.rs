use serde::{Deserialize, Serialize};

use crate::UiRect;

// ---------------------------------------------------------------------------
// Layout — anchor-based positioning
// ---------------------------------------------------------------------------

/// Defines the position and size of a [`UiElement`](crate::UiElement) using
/// anchor points and pixel offsets relative to a parent rectangle (or canvas).
///
/// # Coordinate system
/// * `+X` right, `+Y` down, origin at top‑left of the parent.
/// * Anchors (`anchor_min`, `anchor_max`) are normalised `0..=1` fractions of
///   the parent's width/height.
/// * Offsets (`offset_min`, `offset_max`) are pixel distances from the
///   corresponding anchor corner.
///
/// # Examples
///
/// **Full‑size fill** (anchors span the whole parent):
/// ```ignore
/// Layout {
///     anchor_min: Vec2::new(0.0, 0.0),
///     anchor_max: Vec2::new(1.0, 1.0),
///     offset_min: Vec2::ZERO,
///     offset_max: Vec2::ZERO,
/// }
/// ```
///
/// **Centred 100×50 px box**:
/// ```ignore
/// Layout {
///     anchor_min: Vec2::new(0.5, 0.5),
///     anchor_max: Vec2::new(0.5, 0.5),
///     offset_min: Vec2::new(-50.0, -25.0),
///     offset_max: Vec2::new(50.0, 25.0),
/// }
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// Normalised anchor for the top‑left corner (`0..=1`).
    pub anchor_min: glam::Vec2,
    /// Normalised anchor for the bottom‑right corner (`0..=1`).
    pub anchor_max: glam::Vec2,
    /// Pixel offset from `anchor_min`.
    pub offset_min: glam::Vec2,
    /// Pixel offset from `anchor_max`.
    pub offset_max: glam::Vec2,
}

impl Layout {
    /// Layout that fills the entire parent rectangle.
    pub const FILL: Self = Self {
        anchor_min: glam::Vec2::ZERO,
        anchor_max: glam::Vec2::ONE,
        offset_min: glam::Vec2::ZERO,
        offset_max: glam::Vec2::ZERO,
    };

    /// Create a new layout from explicit anchors and offsets.
    #[inline]
    pub const fn new(
        anchor_min: glam::Vec2,
        anchor_max: glam::Vec2,
        offset_min: glam::Vec2,
        offset_max: glam::Vec2,
    ) -> Self {
        Self {
            anchor_min,
            anchor_max,
            offset_min,
            offset_max,
        }
    }

    /// Resolve this layout into a pixel [`UiRect`] relative to `parent`.
    ///
    /// * `parent` — the containing rectangle (canvas for root elements,
    ///   parent element's computed rect for children).
    pub fn compute(parent: &UiRect, layout: &Self) -> UiRect {
        let x = parent.x + parent.width * layout.anchor_min.x + layout.offset_min.x;
        let y = parent.y + parent.height * layout.anchor_min.y + layout.offset_min.y;

        let w = parent.width * (layout.anchor_max.x - layout.anchor_min.x)
            + (layout.offset_max.x - layout.offset_min.x);
        let h = parent.height * (layout.anchor_max.y - layout.anchor_min.y)
            + (layout.offset_max.y - layout.offset_min.y);

        UiRect::new(x, y, w.max(0.0), h.max(0.0))
    }
}

// ---------------------------------------------------------------------------
// ScaleMode
// ---------------------------------------------------------------------------

/// Controls how a canvas is scaled when the viewport size changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ScaleMode {
    /// No automatic scaling — canvas is always `width × height` pixels.
    #[default]
    Fixed,
    /// Scale to fit the viewport width while preserving aspect ratio.
    FitWidth,
    /// Scale to fit the viewport height while preserving aspect ratio.
    FitHeight,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UiRect;
    use glam::Vec2;

    // -----------------------------------------------------------------------
    // Layout::compute tests
    // -----------------------------------------------------------------------

    #[test]
    fn layout_fill() {
        let parent = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let l = Layout::FILL;
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, parent);
    }

    #[test]
    fn layout_top_left_corner() {
        let parent = UiRect::new(0.0, 0.0, 800.0, 600.0);
        // Anchor to top-left, fixed 100x50 box with 10px margin
        let l = Layout::new(
            Vec2::ZERO,
            Vec2::ZERO,
            Vec2::new(10.0, 10.0),
            Vec2::new(110.0, 60.0),
        );
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, UiRect::new(10.0, 10.0, 100.0, 50.0));
    }

    #[test]
    fn layout_bottom_right_corner() {
        let parent = UiRect::new(0.0, 0.0, 800.0, 600.0);
        // Anchor to bottom-right, fixed 100x50 box with 10px inner margin
        let l = Layout::new(
            Vec2::new(1.0, 1.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(-110.0, -60.0),
            Vec2::new(-10.0, -10.0),
        );
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, UiRect::new(800.0 - 110.0, 600.0 - 60.0, 100.0, 50.0));
    }

    #[test]
    fn layout_centered_fixed() {
        let parent = UiRect::new(0.0, 0.0, 800.0, 600.0);
        // Anchor to centre, 200x100 box
        let l = Layout::new(
            Vec2::new(0.5, 0.5),
            Vec2::new(0.5, 0.5),
            Vec2::new(-100.0, -50.0),
            Vec2::new(100.0, 50.0),
        );
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, UiRect::new(300.0, 250.0, 200.0, 100.0));
    }

    #[test]
    fn layout_stretch_horizontal() {
        let parent = UiRect::new(0.0, 0.0, 800.0, 600.0);
        // Stretch from 25% to 75% horizontally, 50px tall at top
        let l = Layout::new(
            Vec2::new(0.25, 0.0),
            Vec2::new(0.75, 0.0),
            Vec2::new(0.0, 10.0),
            Vec2::new(0.0, 60.0),
        );
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, UiRect::new(200.0, 10.0, 400.0, 50.0));
    }

    #[test]
    fn layout_with_nonzero_parent() {
        let parent = UiRect::new(50.0, 60.0, 200.0, 100.0);
        // Fill half the parent (right half)
        let l = Layout::new(
            Vec2::new(0.5, 0.0),
            Vec2::new(1.0, 1.0),
            Vec2::ZERO,
            Vec2::ZERO,
        );
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, UiRect::new(150.0, 60.0, 100.0, 100.0));
    }

    #[test]
    fn layout_negative_sizes_clamped() {
        let parent = UiRect::new(0.0, 0.0, 100.0, 100.0);
        // Inverted anchors produce negative size; should clamp to zero
        let l = Layout::new(Vec2::ONE, Vec2::ZERO, Vec2::ZERO, Vec2::ZERO);
        let r = Layout::compute(&parent, &l);
        assert_eq!(r.width, 0.0);
        assert_eq!(r.height, 0.0);
    }

    #[test]
    fn layout_identity() {
        let parent = UiRect::new(10.0, 20.0, 30.0, 40.0);
        // anchor_min == anchor_max, offsets zero → zero-size rect at anchor
        let l = Layout::new(
            Vec2::new(0.5, 0.5),
            Vec2::new(0.5, 0.5),
            Vec2::ZERO,
            Vec2::ZERO,
        );
        let r = Layout::compute(&parent, &l);
        assert_eq!(r, UiRect::new(25.0, 40.0, 0.0, 0.0));
    }

    #[test]
    fn scale_mode_default() {
        assert_eq!(ScaleMode::default(), ScaleMode::Fixed);
    }
}
