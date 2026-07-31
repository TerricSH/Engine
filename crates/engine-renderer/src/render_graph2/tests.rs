use super::*;

#[test]
fn custom_pass_names_are_interned_once_across_frame_builds() {
    let PassKind::Custom(first) = PassKind::parse_str("custom_interned").unwrap() else {
        unreachable!();
    };
    let PassKind::Custom(second) = PassKind::parse_str("custom_interned").unwrap() else {
        unreachable!();
    };

    assert!(std::ptr::eq(first, second));
}

fn test_view(view_id: u32) -> RenderView {
    RenderView {
        view_id,
        camera_entity: None,
        viewport: crate::Rect::FULL,
        viewport_rect_normalized: crate::Rect::FULL,
        view_matrix: crate::IDENTITY_MAT4,
        projection_matrix: crate::IDENTITY_MAT4,
        clear_flags: crate::ClearFlags::ColorAndDepth,
        clear_color: [0.0, 0.0, 0.0, 1.0],
        render_layer_mask: u32::MAX,
        msaa_samples: 1,
        compose: ViewCompose::Base {
            clear: crate::ClearFlags::ColorAndDepth,
            clear_color: [0.0, 0.0, 0.0, 1.0],
        },
        stack_order: view_id as i32,
        frustum: None,
    }
}

fn input_with_shadowed_view() -> RenderFrameInput {
    let mut input = RenderFrameInput::empty(0);
    input.views.push(test_view(7));
    input.lights.push(crate::LightItem {
        entity: None,
        kind: crate::LightKind::Directional,
        color: [1.0, 1.0, 1.0],
        intensity: 1.0,
        range: 100.0,
        position: [0.0, 10.0, 0.0],
        direction: [0.0, -1.0, 0.0],
        spot_angles: None,
        shadow_mode: crate::ShadowMode::Hard,
    });
    input
}

// ── Helpers ──────────────────────────────────────────────────────────

fn make_graph() -> RenderGraph {
    // Build a simple 4-pass graph:
    //   A (writes "color") → B (reads "color", writes "temp", writes
    //   "unused") → C (side-effect, reads "temp") → D (reads "temp",
    //   outputs "swapchain")
    let mut graph = RenderGraph::new();

    // Pass 0: A → writes "color"
    graph.add_pass(PassNode {
        kind: PassKind::Custom("A"),
        name: "pass_a",
        view_id: 0,
        inputs: vec![],
        outputs: vec![PassAttachment {
            name: "color".into(),
            format: Some("RGBA16F".into()),
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        }],
        depth_stencil: None,
    });

    // Pass 1: B → reads "color", writes "temp", also writes "unused"
    graph.add_pass(PassNode {
        kind: PassKind::Custom("B"),
        name: "pass_b",
        view_id: 0,
        inputs: vec![PassAttachment {
            name: "color".into(),
            format: Some("RGBA16F".into()),
            clear: false,
            load_op: "load".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Read,
        }],
        outputs: vec![
            PassAttachment {
                name: "temp".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            },
            PassAttachment {
                name: "unused".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            },
        ],
        depth_stencil: None,
    });

    // Pass 2: C → side-effect pass (reads "temp", no outputs)
    graph.add_pass(PassNode {
        kind: PassKind::Custom("C"),
        name: "pass_c",
        view_id: 0,
        inputs: vec![PassAttachment {
            name: "temp".into(),
            format: Some("R8".into()),
            clear: false,
            load_op: "load".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Read,
        }],
        outputs: vec![],
        depth_stencil: None,
    });

    // Pass 3: D → Present (reads "temp", writes "swapchain")
    graph.add_pass(PassNode {
        kind: PassKind::Present,
        name: "present",
        view_id: 0,
        inputs: vec![PassAttachment {
            name: "temp".into(),
            format: Some("R8".into()),
            clear: false,
            load_op: "load".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Read,
        }],
        outputs: vec![PassAttachment {
            name: "swapchain".into(),
            format: None,
            clear: false,
            load_op: "dont_care".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        }],
        depth_stencil: None,
    });

    // Add sequential edges for deterministic ordering.
    let count = graph.passes.len();
    for i in 0..count.saturating_sub(1) {
        graph.add_edge(i, i + 1, "auto");
    }

    graph
}

// ── compile tests ────────────────────────────────────────────────────

include!("tests/config.rs");
include!("tests/compile.rs");
include!("tests/transient.rs");
