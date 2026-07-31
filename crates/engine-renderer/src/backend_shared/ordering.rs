/// Orders transparent work back-to-front when weighted order-independent
/// transparency is unavailable.
///
/// `distance_squared` must return a finite camera-distance key. Keeping this
/// policy in the portable renderer layer prevents backend-specific ordering
/// drift while still allowing each backend to retain its own prepared item.
pub fn order_transparent_back_to_front<T>(
    items: &mut [T],
    weighted_oit: bool,
    distance_squared: impl Fn(&T) -> f32,
) {
    if weighted_oit {
        return;
    }
    items.sort_by(|left, right| distance_squared(right).total_cmp(&distance_squared(left)));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_order_is_back_to_front_without_weighted_oit() {
        let mut items = [(4.0, "middle"), (1.0, "near"), (9.0, "far")];
        order_transparent_back_to_front(&mut items, false, |item| item.0);
        assert_eq!(items.map(|item| item.1), ["far", "middle", "near"]);
    }

    #[test]
    fn weighted_oit_preserves_submission_order() {
        let mut items = [(4.0, "middle"), (1.0, "near"), (9.0, "far")];
        order_transparent_back_to_front(&mut items, true, |item| item.0);
        assert_eq!(items.map(|item| item.1), ["middle", "near", "far"]);
    }
}
