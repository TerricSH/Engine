use std::path::{Path, PathBuf};

fn collect_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let mut entries = std::fs::read_dir(directory)
        .expect("gameplay module directory must be readable")
        .map(|entry| entry.expect("gameplay module entry").path())
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
fn gameplay_contract_files_stay_below_the_module_budget() {
    let source_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sources = vec![source_root.join("gameplay.rs")];
    collect_rust_sources(&source_root.join("gameplay"), &mut sources);

    for source in sources {
        let line_count = std::fs::read_to_string(&source)
            .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()))
            .lines()
            .count();
        assert!(
            line_count < 500,
            "{} grew to {line_count} lines; split the gameplay contract by domain",
            source.display()
        );
    }
}
