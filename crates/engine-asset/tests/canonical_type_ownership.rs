//! Architecture guards for cross-crate rendering and asset contract ownership.

use std::any::TypeId;
use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("engine-asset must live under <workspace>/crates")
        .to_path_buf()
}

fn visit_rust_sources(directory: &Path, sources: &mut Vec<PathBuf>) {
    let mut entries = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("could not read {}: {error}", directory.display()))
        .map(|entry| entry.expect("source entry").path())
        .collect::<Vec<_>>();
    entries.sort();

    for path in entries {
        if path.is_dir() {
            visit_rust_sources(&path, sources);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            sources.push(path);
        }
    }
}

fn definition_sites(declaration: &str) -> Vec<PathBuf> {
    let mut sources = Vec::new();
    let crates = workspace_root().join("crates");
    for entry in fs::read_dir(&crates).expect("workspace crates directory") {
        let source_directory = entry.expect("crate directory entry").path().join("src");
        if source_directory.is_dir() {
            visit_rust_sources(&source_directory, &mut sources);
        }
    }
    sources
        .into_iter()
        .filter(|source| {
            fs::read_to_string(source)
                .unwrap_or_else(|error| panic!("could not read {}: {error}", source.display()))
                .contains(declaration)
        })
        .collect()
}

fn assert_single_owner(declaration: &str, expected_owner: &str) {
    let sites = definition_sites(declaration);
    assert_eq!(
        sites.len(),
        1,
        "{declaration} must have one definition, found: {sites:?}"
    );
    let expected = workspace_root().join(expected_owner);
    assert_eq!(
        sites[0], expected,
        "{declaration} moved away from its canonical owner"
    );
}

#[test]
fn compatibility_paths_preserve_the_canonical_type_identity() {
    assert_eq!(
        TypeId::of::<engine_renderer::IndexFormat>(),
        TypeId::of::<render_core::IndexFormat>()
    );
    assert_eq!(
        TypeId::of::<engine_renderer::LightKind>(),
        TypeId::of::<engine_serialize::LightKind>()
    );
    assert_eq!(
        TypeId::of::<engine_scene::components::LightKind>(),
        TypeId::of::<engine_serialize::LightKind>()
    );
    assert_eq!(
        TypeId::of::<engine_asset::cook::LogicAsset>(),
        TypeId::of::<engine_serialize::LogicAsset>()
    );
}

#[test]
fn canonical_contracts_have_one_source_definition() {
    assert_single_owner("pub enum IndexFormat {", "crates/render-core/src/types.rs");
    assert_single_owner(
        "pub enum LightKind {",
        "crates/engine-serialize/src/lighting.rs",
    );
    assert_single_owner(
        "pub struct LogicAsset {",
        "crates/engine-serialize/src/logic.rs",
    );
}
