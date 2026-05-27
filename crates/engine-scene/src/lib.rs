#![forbid(unsafe_code)]

mod scene;
mod validation;
mod extraction;

pub use scene::*;
pub use validation::validate_scene;
pub use extraction::extract_renderer_input;

#[cfg(test)]
mod tests;
