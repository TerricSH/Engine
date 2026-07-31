use std::path::{Path, PathBuf};

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(directory)
        .expect("editor module directory must be readable")
        .map(|entry| entry.expect("editor module entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            collect_rust_sources(&path, sources);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
}

#[test]
fn large_editor_domains_stay_within_source_budgets() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for module in ["commands", "gizmo", "asset_browser", "prefab_authoring"] {
        let mut sources = vec![source_root.join(format!("{module}.rs"))];
        let module_directory = source_root.join(module);
        assert!(
            module_directory.is_dir(),
            "{module}.rs must remain a facade over owned source fragments"
        );
        collect_rust_sources(&module_directory, &mut sources);

        for source in sources {
            let line_count = std::fs::read_to_string(&source)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()))
                .lines()
                .count();
            let is_test = source
                .components()
                .any(|component| component.as_os_str() == "tests")
                || source.file_name().and_then(|name| name.to_str()) == Some("tests.rs");
            let budget = if is_test { 500 } else { 1_000 };
            assert!(
                line_count < budget,
                "{} grew to {line_count} lines; the {} budget is below {budget}",
                source.display(),
                if is_test {
                    "test-file"
                } else {
                    "production-file"
                },
            );
        }
    }
}
