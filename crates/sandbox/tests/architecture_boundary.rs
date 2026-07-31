//! Architecture guards for the game-script / engine-runtime dependency line.

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("sandbox must live under <workspace>/crates")
        .to_path_buf()
}

fn visit_rust_sources(directory: &Path, files: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
        .map(|entry| entry.expect("source directory entry").path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            visit_rust_sources(&path, files);
        } else if path.extension().and_then(|value| value.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn source_line_count(path: &Path) -> usize {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        .lines()
        .count()
}

fn is_standalone_test_source(path: &Path) -> bool {
    if path
        .components()
        .any(|component| component.as_os_str() == "tests")
    {
        return true;
    }
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name == "tests.rs" || name.ends_with("_tests.rs"))
}

fn cfg_site_count(path: &Path) -> usize {
    fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", path.display()))
        .match_indices("#[cfg")
        .count()
}

fn read_module_tree(root_file: &Path) -> String {
    let mut source = fs::read_to_string(root_file)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", root_file.display()));
    let module_dir = root_file.with_extension("");
    if module_dir.is_dir() {
        let mut children = Vec::new();
        visit_rust_sources(&module_dir, &mut children);
        for child in children {
            source.push('\n');
            source.push_str(
                &fs::read_to_string(&child)
                    .unwrap_or_else(|error| panic!("could not read {}: {error}", child.display())),
            );
        }
    }
    source
}

fn assert_facade_budget(path: &Path, max_lines: usize, max_cfg_sites: usize) {
    let lines = source_line_count(path);
    let cfg_sites = cfg_site_count(path);
    assert!(
        lines <= max_lines,
        "{} grew to {lines} lines; keep domain implementation in its child modules",
        path.display()
    );
    assert!(
        cfg_sites <= max_cfg_sites,
        "{} grew to {cfg_sites} cfg sites; keep feature branches with their owning adapter",
        path.display()
    );
}

include!("architecture_boundary/sources_and_dependencies.rs");
include!("architecture_boundary/composition_facades.rs");
include!("architecture_boundary/platform_and_dead_code.rs");
include!("architecture_boundary/retired_paths.rs");
include!("architecture_boundary/editor_and_render_graph.rs");
