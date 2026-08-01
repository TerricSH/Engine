use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum XrHand {
    #[default]
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum XrActionValue {
    Boolean(bool),
    Float(f32),
    Vector2([f32; 2]),
    Pose(super::XrPose),
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct XrActionSnapshot {
    pub values: BTreeMap<String, XrActionValue>,
}

impl XrActionSnapshot {
    pub fn boolean(&self, action: &str) -> Option<bool> {
        match self.values.get(action) {
            Some(XrActionValue::Boolean(value)) => Some(*value),
            _ => None,
        }
    }

    pub fn scalar(&self, action: &str) -> Option<f32> {
        match self.values.get(action) {
            Some(XrActionValue::Float(value)) => Some(*value),
            _ => None,
        }
    }
}
