#![forbid(unsafe_code)]

mod skeleton;
mod pose;
mod clip;
mod player;

pub use skeleton::{BoneIndex, BoneTransform, AnimationError, Skeleton};
pub use pose::Pose;
pub use clip::{Keyframe, AnimationClip};
pub use player::{AnimationPlayer, Animator};

#[cfg(test)]
mod tests;
