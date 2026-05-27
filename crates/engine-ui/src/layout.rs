use crate::UiRect;

// ---------------------------------------------------------------------------
// Layout helper functions
// ---------------------------------------------------------------------------

/// Align `rect` to the left edge of `container` with `padding`.
pub fn align_left(rect: &UiRect, container: &UiRect, padding: f32) -> UiRect {
    UiRect::new(
        container.x + padding,
        rect.y,
        rect.width,
        rect.height,
    )
}

/// Align `rect` to the right edge of `container` with `padding`.
pub fn align_right(rect: &UiRect, container: &UiRect, padding: f32) -> UiRect {
    UiRect::new(
        container.x + container.width - rect.width - padding,
        rect.y,
        rect.width,
        rect.height,
    )
}

/// Align `rect` to the top edge of `container` with `padding`.
pub fn align_top(rect: &UiRect, container: &UiRect, padding: f32) -> UiRect {
    UiRect::new(
        rect.x,
        container.y + padding,
        rect.width,
        rect.height,
    )
}

/// Align `rect` to the bottom edge of `container` with `padding`.
pub fn align_bottom(rect: &UiRect, container: &UiRect, padding: f32) -> UiRect {
    UiRect::new(
        rect.x,
        container.y + container.height - rect.height - padding,
        rect.width,
        rect.height,
    )
}

/// Center `rect` horizontally within `container`.
pub fn center_horizontal(rect: &UiRect, container: &UiRect) -> UiRect {
    UiRect::new(
        container.x + (container.width - rect.width) * 0.5,
        rect.y,
        rect.width,
        rect.height,
    )
}

/// Center `rect` vertically within `container`.
pub fn center_vertical(rect: &UiRect, container: &UiRect) -> UiRect {
    UiRect::new(
        rect.x,
        container.y + (container.height - rect.height) * 0.5,
        rect.width,
        rect.height,
    )
}

/// Place `rect` above `target` with `spacing` pixels between them.
pub fn place_above(rect: &UiRect, target: &UiRect, spacing: f32) -> UiRect {
    UiRect::new(
        rect.x,
        target.y - rect.height - spacing,
        rect.width,
        rect.height,
    )
}

/// Place `rect` below `target` with `spacing` pixels between them.
pub fn place_below(rect: &UiRect, target: &UiRect, spacing: f32) -> UiRect {
    UiRect::new(
        rect.x,
        target.y + target.height + spacing,
        rect.width,
        rect.height,
    )
}

/// Place `rect` to the right of `target` with `spacing` pixels between them.
pub fn place_right_of(rect: &UiRect, target: &UiRect, spacing: f32) -> UiRect {
    UiRect::new(
        target.x + target.width + spacing,
        rect.y,
        rect.width,
        rect.height,
    )
}

/// Place `rect` to the left of `target` with `spacing` pixels between them.
pub fn place_left_of(rect: &UiRect, target: &UiRect, spacing: f32) -> UiRect {
    UiRect::new(
        target.x - rect.width - spacing,
        rect.y,
        rect.width,
        rect.height,
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::UiRect;

    #[test]
    fn layout_align_left() {
        let container = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let r = UiRect::new(0.0, 0.0, 50.0, 30.0);
        let result = align_left(&r, &container, 10.0);
        assert_eq!(result.x, 110.0);
        assert_eq!(result.y, 0.0);
    }

    #[test]
    fn layout_align_right() {
        let container = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let r = UiRect::new(0.0, 0.0, 50.0, 30.0);
        let result = align_right(&r, &container, 10.0);
        assert_eq!(result.x, 100.0 + 800.0 - 50.0 - 10.0);
    }

    #[test]
    fn layout_align_top() {
        let container = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let r = UiRect::new(0.0, 0.0, 50.0, 30.0);
        let result = align_top(&r, &container, 15.0);
        assert_eq!(result.y, 215.0);
    }

    #[test]
    fn layout_align_bottom() {
        let container = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let r = UiRect::new(0.0, 0.0, 50.0, 30.0);
        let result = align_bottom(&r, &container, 15.0);
        assert_eq!(result.y, 200.0 + 600.0 - 30.0 - 15.0);
    }

    #[test]
    fn layout_center_horizontal() {
        let container = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let r = UiRect::new(0.0, 0.0, 50.0, 30.0);
        let result = center_horizontal(&r, &container);
        assert_eq!(result.x, 100.0 + (800.0 - 50.0) * 0.5);
    }

    #[test]
    fn layout_center_vertical() {
        let container = UiRect::new(100.0, 200.0, 800.0, 600.0);
        let r = UiRect::new(0.0, 0.0, 50.0, 30.0);
        let result = center_vertical(&r, &container);
        assert_eq!(result.y, 200.0 + (600.0 - 30.0) * 0.5);
    }

    #[test]
    fn layout_place_above() {
        let target = UiRect::new(50.0, 100.0, 200.0, 50.0);
        let r = UiRect::new(0.0, 0.0, 100.0, 30.0);
        let result = place_above(&r, &target, 8.0);
        assert_eq!(result.y, 100.0 - 30.0 - 8.0);
    }

    #[test]
    fn layout_place_below() {
        let target = UiRect::new(50.0, 100.0, 200.0, 50.0);
        let r = UiRect::new(0.0, 0.0, 100.0, 30.0);
        let result = place_below(&r, &target, 8.0);
        assert_eq!(result.y, 100.0 + 50.0 + 8.0);
    }

    #[test]
    fn layout_place_right_of() {
        let target = UiRect::new(50.0, 100.0, 200.0, 50.0);
        let r = UiRect::new(0.0, 0.0, 100.0, 30.0);
        let result = place_right_of(&r, &target, 8.0);
        assert_eq!(result.x, 50.0 + 200.0 + 8.0);
    }

    #[test]
    fn layout_place_left_of() {
        let target = UiRect::new(50.0, 100.0, 200.0, 50.0);
        let r = UiRect::new(0.0, 0.0, 100.0, 30.0);
        let result = place_left_of(&r, &target, 8.0);
        assert_eq!(result.x, 50.0 - 100.0 - 8.0);
    }

    #[test]
    fn layout_align_left_preserves_y_and_size() {
        let container = UiRect::new(0.0, 0.0, 800.0, 600.0);
        let r = UiRect::new(100.0, 200.0, 50.0, 30.0);
        let result = align_left(&r, &container, 0.0);
        assert_eq!(result.x, 0.0);
        assert_eq!(result.y, 200.0);
        assert_eq!(result.width, 50.0);
        assert_eq!(result.height, 30.0);
    }
}
