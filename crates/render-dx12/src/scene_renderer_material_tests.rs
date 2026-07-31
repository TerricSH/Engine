use super::*;
use crate::{DirectX12Backend, Dx12Device};
use engine_renderer::{ClearFlags, LightItem, LightKind, Rect, RenderView, ViewCompose};
use render_core::{Backend, DeviceDescriptor, ValidationMode};

include!("scene_renderer_material_tests/constants.rs");
include!("scene_renderer_material_tests/rendering.rs");
