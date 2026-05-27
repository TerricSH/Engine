#![forbid(unsafe_code)]

mod world;
mod types;
mod convert;

pub use world::PhysicsWorld;
pub use types::*;
pub use convert::{from_rapier_vec, to_rapier_vec};

#[cfg(test)]
mod tests;
