use super::*;
use engine_renderer::{RenderableItem, SkinnedItem};
use render_core::PipelineHandle;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

include!("scene_renderer_tests/uploads.rs");
include!("scene_renderer_tests/custom_passes.rs");
include!("scene_renderer_tests/frame_contract.rs");
include!("scene_renderer_tests/tone_map.rs");
