use serde::{Deserialize, Serialize};

/// Portable light-source classification shared by scene data and render input.
///
/// Keeping this enum in the serialization contract prevents scene extraction
/// from maintaining a second, exhaustively mapped copy of the same concept.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LightKind {
    Directional,
    Point,
    Spot,
}

#[cfg(test)]
mod tests {
    use super::LightKind;

    #[test]
    fn light_kind_json_contract_is_stable() {
        let cases = [
            (LightKind::Directional, "\"Directional\""),
            (LightKind::Point, "\"Point\""),
            (LightKind::Spot, "\"Spot\""),
        ];

        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).expect("serialize light kind");
            assert_eq!(json, expected);
            assert_eq!(
                serde_json::from_str::<LightKind>(&json).expect("deserialize light kind"),
                kind
            );
        }
    }
}
