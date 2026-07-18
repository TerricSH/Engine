use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;
use sha2::{Digest, Sha256};

const FRONTEND_ROOT_FILES: &[&str] = &[
    "editor-web/index.html",
    "editor-web/package.json",
    "editor-web/pnpm-lock.yaml",
    "editor-web/pnpm-workspace.yaml",
    "editor-web/tsconfig.app.json",
    "editor-web/tsconfig.json",
    "editor-web/tsconfig.node.json",
    "editor-web/vite.config.ts",
    "src/editor_app/dispatch.rs",
    "src/editor_app/protocol.rs",
    "src/editor_app/snapshot.rs",
];

const FRONTEND_DIRECTORIES: &[&str] = &["editor-web/scripts", "editor-web/src"];

const DIST_ASSETS: &[&str] = &[
    "editor-web/dist/index.html",
    "editor-web/dist/assets/editor.js",
    "editor-web/dist/assets/editor.css",
];

fn main() {
    println!("cargo:rerun-if-env-changed=ENGINE_EDITOR_WEB_DEV_URL");
    for path in FRONTEND_ROOT_FILES
        .iter()
        .chain(FRONTEND_DIRECTORIES.iter())
    {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-changed=editor-web/dist/build-manifest.json");
    for path in DIST_ASSETS {
        println!("cargo:rerun-if-changed={path}");
    }

    if env::var_os("CARGO_FEATURE_TOOLING_EDITOR").is_none() {
        return;
    }
    if env::var("PROFILE").as_deref() == Ok("debug")
        && env::var_os("ENGINE_EDITOR_WEB_DEV_URL").is_some()
    {
        println!(
            "cargo:warning=React dist freshness check skipped for the explicit loopback development server"
        );
        return;
    }

    verify_frontend_bundle().unwrap_or_else(|error| {
        panic!(
            "React editor production bundle is missing or stale: {error}. Run `npm run build` in crates/sandbox/editor-web"
        )
    });
}

fn verify_frontend_bundle() -> Result<(), String> {
    let manifest_dir = PathBuf::from(
        env::var_os("CARGO_MANIFEST_DIR")
            .ok_or_else(|| "CARGO_MANIFEST_DIR is unavailable".to_string())?,
    );
    let manifest_path = manifest_dir.join("editor-web/dist/build-manifest.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let manifest: Value = serde_json::from_slice(&manifest_bytes)
        .map_err(|error| format!("cannot decode {}: {error}", manifest_path.display()))?;
    if manifest.get("schema").and_then(Value::as_u64) != Some(1) {
        return Err("dist manifest has an unsupported schema".into());
    }

    let expected_source_hash = manifest
        .get("sourceHash")
        .and_then(Value::as_str)
        .ok_or_else(|| "dist manifest has no sourceHash".to_string())?;
    let source_hash = hash_frontend_sources(&manifest_dir)?;
    if source_hash != expected_source_hash {
        return Err(format!(
            "source hash {source_hash} does not match bundled hash {expected_source_hash}"
        ));
    }

    let asset_hashes = manifest
        .get("assets")
        .and_then(Value::as_object)
        .ok_or_else(|| "dist manifest has no asset hash table".to_string())?;
    for relative in DIST_ASSETS {
        let manifest_key = relative
            .strip_prefix("editor-web/dist/")
            .expect("dist asset paths use the canonical prefix");
        let expected = asset_hashes
            .get(manifest_key)
            .and_then(Value::as_str)
            .ok_or_else(|| format!("dist manifest has no hash for {manifest_key}"))?;
        let actual = hash_file(&manifest_dir.join(relative))?;
        if actual != expected {
            return Err(format!(
                "bundled asset {manifest_key} hash {actual} does not match manifest hash {expected}"
            ));
        }
    }
    Ok(())
}

fn hash_frontend_sources(manifest_dir: &Path) -> Result<String, String> {
    let mut inputs = FRONTEND_ROOT_FILES
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<_>>();
    for directory in FRONTEND_DIRECTORIES {
        collect_files(manifest_dir, Path::new(directory), &mut inputs)?;
    }
    inputs.sort_by_key(|path| normalized(path));
    inputs.dedup();

    let mut hash = Sha256::new();
    for relative in inputs {
        let normalized = normalized(&relative);
        let bytes = fs::read(manifest_dir.join(&relative)).map_err(|error| {
            format!("cannot read frontend input {}: {error}", relative.display())
        })?;
        hash.update(normalized.as_bytes());
        hash.update([0]);
        hash.update(&bytes);
        hash.update([0]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn collect_files(root: &Path, relative: &Path, output: &mut Vec<PathBuf>) -> Result<(), String> {
    let directory = root.join(relative);
    let entries = fs::read_dir(&directory)
        .map_err(|error| format!("cannot scan {}: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("cannot scan {}: {error}", directory.display()))?;
        let child = relative.join(entry.file_name());
        let file_type = entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", entry.path().display()))?;
        if file_type.is_dir() {
            collect_files(root, &child, output)?;
        } else if file_type.is_file() {
            output.push(child);
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<String, String> {
    let bytes =
        fs::read(path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
