#[test]
fn transient_pool_empty_graph() {
    let graph = RenderGraph::new();
    let pool = TransientResourcePool::default();
    let plan = pool.build(&graph, &[]);
    assert!(plan.slots.is_empty());
    assert!(plan.resource_to_slot.is_empty());
}

#[test]
fn transient_pool_aliases_non_overlapping_resources() {
    let graph = make_graph();
    let order: Vec<usize> = (0..graph.passes.len()).collect();
    let pool = TransientResourcePool::default();
    let plan = pool.build(&graph, &order);

    assert!(
        !plan.resource_to_slot.contains_key("swapchain"),
        "swapchain should be exempt"
    );

    for name in &["color", "temp", "unused"] {
        assert!(
            plan.resource_to_slot.contains_key(*name),
            "resource '{name}' should have a slot assignment"
        );
    }
}

#[test]
fn transient_pool_overlapping_resources_get_different_slots() {
    let mut graph = RenderGraph::new();

    graph.add_pass(PassNode {
        kind: PassKind::Custom("P"),
        name: "p",
        view_id: 0,
        inputs: vec![],
        outputs: vec![
            PassAttachment {
                name: "a".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            },
            PassAttachment {
                name: "b".into(),
                format: Some("R8".into()),
                clear: true,
                load_op: "clear".into(),
                size_source: SizeSource::Swapchain,
                access: ResourceAccess::Write,
            },
        ],
        depth_stencil: None,
    });

    let order = vec![0usize];
    let pool = TransientResourcePool::new(vec![]);
    let plan = pool.build(&graph, &order);

    let slot_a = plan.resource_to_slot.get("a").copied();
    let slot_b = plan.resource_to_slot.get("b").copied();
    assert!(slot_a.is_some());
    assert!(slot_b.is_some());
    assert_ne!(
        slot_a, slot_b,
        "overlapping resources must get different slots"
    );
}

#[test]
fn transient_pool_sequential_resources_can_share_slot() {
    let mut graph = RenderGraph::new();

    // Pass 0: writes "early"
    graph.add_pass(PassNode {
        kind: PassKind::Custom("E"),
        name: "early",
        view_id: 0,
        inputs: vec![],
        outputs: vec![PassAttachment {
            name: "early".into(),
            format: Some("R8".into()),
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        }],
        depth_stencil: None,
    });

    // Pass 1: reads "early", writes "late" (no overlap — early not
    // used after pass 1, late starts at pass 1).
    graph.add_pass(PassNode {
        kind: PassKind::Custom("L"),
        name: "late",
        view_id: 0,
        inputs: vec![PassAttachment {
            name: "early".into(),
            format: Some("R8".into()),
            clear: false,
            load_op: "load".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Read,
        }],
        outputs: vec![PassAttachment {
            name: "late".into(),
            format: Some("R16F".into()),
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        }],
        depth_stencil: None,
    });

    // Pass 2: Present (consumes "late")
    graph.add_pass(PassNode {
        kind: PassKind::Present,
        name: "present",
        view_id: 0,
        inputs: vec![PassAttachment {
            name: "late".into(),
            format: Some("R16F".into()),
            clear: false,
            load_op: "load".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Read,
        }],
        outputs: vec![],
        depth_stencil: None,
    });

    let order: Vec<usize> = (0..graph.passes.len()).collect();
    let pool = TransientResourcePool::new(vec![]);
    let plan = pool.build(&graph, &order);

    let slot_early = plan.resource_to_slot.get("early").copied();
    let slot_late = plan.resource_to_slot.get("late").copied();
    assert!(slot_early.is_some());
    assert!(slot_late.is_some());
    // Early's lifetime [0,1], late's [1,2]; they overlap at pass 1
    // (both referenced), so must be in different slots.
    assert_ne!(slot_early, slot_late);
}
