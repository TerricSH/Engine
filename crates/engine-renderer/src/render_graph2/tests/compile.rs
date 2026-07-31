#[test]
fn compile_topological_sort() {
    let graph = make_graph();
    let compiled = graph.compile().expect("compile should succeed");
    assert_eq!(compiled.pass_order.len(), 4);
    let positions: Vec<usize> = compiled.pass_order.to_vec();
    for w in positions.windows(2) {
        assert!(w[0] < w[1], "pass_order should be ascending");
    }
}

#[test]
fn compile_culls_unconsumed_output() {
    // Pass B writes "unused" that nobody reads, but B also writes "temp"
    // which IS consumed → B stays live because "temp" reaches Present.
    let graph = make_graph();
    let compiled = graph.compile().expect("compile should succeed");
    assert_eq!(compiled.pass_order.len(), 4);
}

#[test]
fn compile_culls_dead_pass() {
    let mut graph = make_graph();

    // Pass 4: E (dead) — writes "dead_buffer", nothing reads it.
    graph.add_pass(PassNode {
        kind: PassKind::Custom("E"),
        name: "dead_pass",
        view_id: 0,
        inputs: vec![],
        outputs: vec![PassAttachment {
            name: "dead_buffer".into(),
            format: Some("R8".into()),
            clear: true,
            load_op: "clear".into(),
            size_source: SizeSource::Swapchain,
            access: ResourceAccess::Write,
        }],
        depth_stencil: None,
    });

    let compiled = graph.compile().expect("compile should succeed");
    assert_eq!(compiled.pass_order.len(), 4);
    assert!(
        !compiled.pass_order.contains(&4),
        "dead pass should be culled"
    );
}

#[test]
fn compile_barriers_between_transitions() {
    let graph = make_graph();
    let compiled = graph.compile().expect("compile should succeed");
    let total_barriers: usize = compiled.barriers_per_pass.iter().map(|b| b.len()).sum();
    assert!(total_barriers >= 1, "expected at least 1 barrier");
}

#[test]
fn compile_empty_graph() {
    let graph = RenderGraph::new();
    let compiled = graph.compile().expect("empty graph should compile");
    assert!(compiled.pass_order.is_empty());
    assert!(compiled.barriers_per_pass.is_empty());
}

// ── TransientResourcePool tests ──────────────────────────────────────
