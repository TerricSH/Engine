#[test]
fn default_config_uses_canonical_builtin_resource_declarations() {
    let input = input_with_shadowed_view();
    let canonical = RenderGraph::build(&input);
    let configured = RenderGraph::build_with_config(&input, &PassGraphConfig::default());

    assert_eq!(configured.passes, canonical.passes);
    assert_eq!(configured.edges.len(), canonical.edges.len());
    assert!(configured.edges.iter().all(|edge| {
        configured.passes[edge.from_pass].view_id == configured.passes[edge.to_pass].view_id
    }));
}

#[test]
fn default_config_compile_transitions_hdr_for_tone_mapping() {
    let input = input_with_shadowed_view();
    let graph = RenderGraph::build_with_config(&input, &PassGraphConfig::default());
    let compiled = graph.compile().expect("default graph should compile");
    let tone_map = graph
        .passes
        .iter()
        .find(|pass| matches!(pass.kind, PassKind::ToneMap))
        .expect("tone-map pass");
    let present = graph
        .passes
        .iter()
        .find(|pass| matches!(pass.kind, PassKind::Present))
        .expect("present pass");
    assert_eq!(
        tone_map
            .inputs
            .iter()
            .map(|attachment| attachment.name.as_str())
            .collect::<Vec<_>>(),
        vec!["hdr_color", "oit_accumulation", "oit_optical_depth"]
    );
    assert_eq!(tone_map.outputs[0].name, "swapchain");
    assert_eq!(present.inputs[0].name, "swapchain");
    assert!(present.outputs.is_empty());
    let tone_map_position = compiled
        .pass_order
        .iter()
        .position(|&pass_idx| matches!(graph.passes[pass_idx].kind, PassKind::ToneMap))
        .expect("tone-map pass should remain live");

    assert!(compiled.barriers_per_pass[tone_map_position]
        .iter()
        .any(|barrier| {
            barrier.resource_name == "hdr_color"
                && barrier.old_state == ResourceState::ColorAttachmentOptimal
                && barrier.new_state == ResourceState::ShaderReadOnlyOptimal
                && barrier.src_stage == PipeStage::ColorAttachmentOutput
                && barrier.dst_stage == PipeStage::FragmentShader
        }));
    for resource_name in ["oit_accumulation", "oit_optical_depth"] {
        assert!(compiled.barriers_per_pass[tone_map_position]
            .iter()
            .any(|barrier| {
                barrier.resource_name == resource_name
                    && barrier.old_state == ResourceState::ColorAttachmentOptimal
                    && barrier.new_state == ResourceState::ShaderReadOnlyOptimal
            }));
    }
}

#[test]
fn direct_to_swapchain_declares_opaque_and_present_resource_flow() {
    let mut input = RenderFrameInput::empty(0);
    input.views.push(test_view(3));
    let config = PassGraphConfig {
        output_mode: PassGraphOutputMode::DirectToSwapchain,
        ..PassGraphConfig::default()
    };

    let graph = RenderGraph::build_with_config(&input, &config);
    let kinds: Vec<&'static str> = graph.passes.iter().map(|pass| pass.kind.name()).collect();
    assert_eq!(kinds, vec!["opaque_pbr_forward_pass", "present"]);

    let opaque = &graph.passes[0];
    assert_eq!(
        opaque
            .outputs
            .iter()
            .map(|attachment| attachment.name.as_str())
            .collect::<Vec<_>>(),
        vec!["swapchain"]
    );
    assert_eq!(
        opaque
            .depth_stencil
            .as_ref()
            .map(|depth| depth.name.as_str()),
        Some("depth_stencil")
    );
    assert_eq!(
        opaque.depth_stencil.as_ref().map(|depth| depth.access),
        Some(ResourceAccess::ReadWrite)
    );

    let present = &graph.passes[1];
    assert_eq!(
        present
            .inputs
            .iter()
            .map(|attachment| attachment.name.as_str())
            .collect::<Vec<_>>(),
        vec!["swapchain"]
    );
    assert!(present.outputs.is_empty());
    assert_eq!(graph.edges.len(), 1);
    assert_eq!(graph.edges[0].from_pass, 0);
    assert_eq!(graph.edges[0].to_pass, 1);

    let compiled = graph.compile().expect("direct graph should compile");
    let present_position = compiled
        .pass_order
        .iter()
        .position(|&pass_idx| matches!(graph.passes[pass_idx].kind, PassKind::Present))
        .expect("present should remain live");
    assert!(compiled.barriers_per_pass[present_position]
        .iter()
        .any(|barrier| {
            barrier.resource_name == "swapchain"
                && barrier.old_state == ResourceState::ColorAttachmentOptimal
                && barrier.new_state == ResourceState::PresentSrc
        }));
}

#[test]
fn builtin_multi_view_edges_never_cross_view_boundaries() {
    let mut input = RenderFrameInput::empty(0);
    input.views = vec![test_view(10), test_view(20)];
    let graphs = [
        RenderGraph::build(&input),
        RenderGraph::build_with_config(&input, &PassGraphConfig::default()),
    ];

    for graph in graphs {
        assert_eq!(graph.passes.len(), 6);
        assert_eq!(graph.edges.len(), 4);
        assert!(graph.edges.iter().all(|edge| {
            graph.passes[edge.from_pass].view_id == graph.passes[edge.to_pass].view_id
        }));
    }
}
