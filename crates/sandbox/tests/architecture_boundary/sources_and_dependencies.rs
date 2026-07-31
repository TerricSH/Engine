#[test]
fn production_rust_files_stay_below_the_giant_file_threshold() {
    let crates = workspace_root().join("crates");
    let mut oversized = Vec::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let source_dir = entry.expect("crate directory entry").path().join("src");
        if !source_dir.is_dir() {
            continue;
        }
        let mut sources = Vec::new();
        visit_rust_sources(&source_dir, &mut sources);
        oversized.extend(
            sources
                .into_iter()
                .filter(|source| !is_standalone_test_source(source))
                .filter_map(|source| {
                    let lines = source_line_count(&source);
                    (lines >= 1_000).then_some((source, lines))
                }),
        );
    }
    oversized.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(
        oversized.is_empty(),
        "production Rust files reached the 1,000-line giant-file threshold: {oversized:#?}"
    );
}

#[test]
fn standalone_rust_test_files_stay_below_500_lines() {
    let crates = workspace_root().join("crates");
    let mut sources = Vec::new();
    visit_rust_sources(&crates, &mut sources);

    let mut oversized = sources
        .into_iter()
        .filter(|source| is_standalone_test_source(source))
        .filter_map(|source| {
            let lines = source_line_count(&source);
            (lines >= 500).then_some((source, lines))
        })
        .collect::<Vec<_>>();
    oversized.sort_by(|left, right| left.0.cmp(&right.0));

    assert!(
        oversized.is_empty(),
        "standalone Rust test files must stay below 500 lines; split fixtures and domains into child test modules: {oversized:#?}"
    );
}

#[test]
fn engine_crates_do_not_depend_on_the_sandbox_application() {
    let crates = workspace_root().join("crates");
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let crate_dir = entry.expect("crate directory entry").path();
        if !crate_dir.is_dir()
            || crate_dir.file_name().and_then(|name| name.to_str()) == Some("sandbox")
        {
            continue;
        }
        let manifest = crate_dir.join("Cargo.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = fs::read_to_string(&manifest).expect("crate manifest");
        assert!(
            !text.lines().any(|line| {
                let compact = line.trim_start();
                compact.starts_with("sandbox =") || compact.contains("path = \"../sandbox\"")
            }),
            "engine crate {} must not depend on the sandbox application",
            crate_dir.display()
        );
    }
}

#[test]
fn production_engine_sources_do_not_reference_example_game_content() {
    let crates = workspace_root().join("crates");
    let mut sources = Vec::new();
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let source_dir = entry.expect("crate directory entry").path().join("src");
        if source_dir.is_dir() {
            visit_rust_sources(&source_dir, &mut sources);
        }
    }

    for source in sources {
        let text = fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()));
        assert!(
            !text.contains("examples/minimal-game") && !text.contains(r"examples\minimal-game"),
            "production source {} must consume a project path, not the repository example game",
            source.display()
        );
    }
}

#[test]
fn script_api_contract_crate_remains_data_only() {
    let manifest = workspace_root().join("crates/engine-script-api/Cargo.toml");
    let text = fs::read_to_string(&manifest).expect("engine-script-api manifest");
    let dependencies = text
        .split_once("[dependencies]")
        .map(|(_, value)| value.trim())
        .unwrap_or_default();
    assert!(
        dependencies.is_empty(),
        "engine-script-api must not acquire runtime, renderer, ECS, editor, or platform dependencies"
    );
}
