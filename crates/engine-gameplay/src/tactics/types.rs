use serde::{Deserialize, Serialize};

/// Stable identifier used by the tactical domain. The engine deliberately does
/// not prescribe how a game maps scene entities to tactical entities.
pub type TacticalEntityId = String;

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct FactionId(pub u16);

#[derive(
    Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
pub struct ActionId(pub u64);
