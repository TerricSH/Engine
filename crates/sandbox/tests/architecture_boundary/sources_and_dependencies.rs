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

#[test]
fn retired_direct_dependencies_and_ambiguous_script_module_cannot_reenter() {
    let root = workspace_root();
    for (crate_name, retired) in [
        (
            "engine-messaging",
            &["engine-serialize", "serde", "tracing"][..],
        ),
        ("engine-script", &["crossbeam-channel"][..]),
        ("engine-scene", &["indexmap"][..]),
        ("engine-gameplay", &["engine-serialize"][..]),
        ("engine-hot-update", &["crossbeam-channel"][..]),
        ("engine-ffi", &["engine-serialize"][..]),
        ("engine-editor", &["engine-script", "sha2"][..]),
        (
            "sandbox",
            &["render-core", "render-opengl", "render-dx12"][..],
        ),
    ] {
        let manifest = fs::read_to_string(root.join("crates").join(crate_name).join("Cargo.toml"))
            .unwrap_or_else(|error| panic!("could not read {crate_name} manifest: {error}"));
        let direct_dependencies = manifest
            .split_once("[dependencies]")
            .map(|(_, dependencies)| dependencies)
            .unwrap_or_default()
            .split('\n')
            .take_while(|line| !line.trim_start().starts_with('['))
            .filter_map(|line| line.split_once('=').map(|(name, _)| name.trim()))
            .collect::<Vec<_>>();
        for dependency in retired {
            assert!(
                !direct_dependencies.contains(dependency),
                "{crate_name} reintroduced retired direct dependency '{dependency}'"
            );
        }
    }

    let sandbox_manifest =
        fs::read_to_string(root.join("crates/sandbox/Cargo.toml")).expect("sandbox manifest");
    for retired_feature in ["backend-opengl =", "backend-dx12 ="] {
        assert!(
            !sandbox_manifest.contains(retired_feature),
            "sandbox must not advertise a backend feature without a runtime adapter: {retired_feature}"
        );
    }
    for crate_name in ["engine-vfx", "engine-procgen", "engine-terrain"] {
        let manifest = fs::read_to_string(root.join("crates").join(crate_name).join("Cargo.toml"))
            .unwrap_or_else(|error| panic!("could not read {crate_name} manifest: {error}"));
        let features = manifest
            .split_once("[features]")
            .map(|(_, features)| features)
            .unwrap_or_default()
            .split('\n')
            .take_while(|line| !line.trim_start().starts_with('['))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            features.lines().any(|line| line.trim() == "default = []"),
            "{crate_name} must declare an explicit empty default feature set"
        );
    }
    let editor_manifest =
        fs::read_to_string(root.join("crates/engine-editor/Cargo.toml")).expect("editor manifest");
    assert!(
        editor_manifest.contains("dep:engine-audio"),
        "engine-editor features must use explicit dep: syntax for optional audio"
    );

    let scripts = root.join("crates/sandbox/src/project_scripts");
    assert!(!scripts.join("build.rs").exists());
    assert!(scripts.join("compilation.rs").is_file());
    let facade = fs::read_to_string(root.join("crates/sandbox/src/project_scripts.rs"))
        .expect("project scripts facade");
    assert!(facade.contains("mod compilation;"));
    assert!(!facade.contains("mod build;"));
}
