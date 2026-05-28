use crate::Component;
use serde::{Deserialize, Serialize};

/// A human-readable name for an entity.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Name(pub String);

impl Component for Name {
    const TYPE_ID: &'static str = "engine.name";
}
