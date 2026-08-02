//! C# ↔ Rust ProcGen parity gate (ENG-10).
//!
//! The managed `Engine.ProcGen` API in the gameplay SDK is a bit-exact port
//! of the native `engine-procgen` crate. This test materializes the
//! engine-owned SDK source via `sandbox project sync-script-api`, compiles it
//! together with `scripts/csharp/ProcGenParity/Program.cs`, and runs the
//! harness against the same checked-in golden vectors that gate the Rust
//! implementation — identical `f32` bit patterns in both languages.
//!
//! The test requires the .NET SDK; on machines without it the test reports a
//! capability SKIP (see `tests/common/mod.rs`) instead of failing.

#![cfg(feature = "subsystem-scripting-csharp")]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

fn sandbox() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_sandbox"))
}

fn run_sandbox(arguments: &[&str]) -> Output {
    Command::new(sandbox())
        .args(arguments)
        .env("ENGINE_LOG_DIR", "off")
        .output()
        .expect("run sandbox")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("sandbox crate lives under <workspace>/crates")
        .to_path_buf()
}

fn path_text(path: &Path) -> &str {
    path.to_str().expect("test path is UTF-8")
}

fn assert_success(output: &Output, operation: &str) {
    assert!(
        output.status.success(),
        "{operation} failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(feature = "subsystem-scripting-csharp")]
#[test]
fn csharp_procgen_port_matches_rust_golden_vectors() {
    if !common::require_tool("dotnet") {
        return;
    }
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "sandbox-procgen-parity-{}-{unique}",
        std::process::id()
    ));

    let output = run_sandbox(&[
        "project",
        "new",
        path_text(&root),
        "--name",
        "ProcGen Parity",
        "--with-csharp",
    ]);
    assert_success(&output, "parity project new");
    let output = run_sandbox(&["project", "sync-script-api", path_text(&root)]);
    assert_success(&output, "parity sync-script-api");

    let sdk_source = root
        .join("build/script-sdk-source")
        .join(engine_script_api::GENERATED_CSHARP_API_FILE);
    assert!(sdk_source.is_file(), "SDK source materialized by sync");
    let runtime_assets_source = root
        .join("build/script-sdk-source")
        .join(engine_script_api::GENERATED_CSHARP_RUNTIME_ASSETS_FILE);
    assert!(
        runtime_assets_source.is_file(),
        "runtime asset SDK source materialized by sync"
    );
    let online_xr_source = root
        .join("build/script-sdk-source")
        .join(engine_script_api::GENERATED_CSHARP_ONLINE_XR_FILE);
    assert!(
        online_xr_source.is_file(),
        "online/XR SDK source materialized by sync"
    );
    let harness_source = workspace_root().join("scripts/csharp/ProcGenParity/Program.cs");
    assert!(
        harness_source.is_file(),
        "parity harness source is checked in"
    );
    let golden_vectors = workspace_root().join("crates/engine-procgen/tests/golden_vectors.json");
    assert!(golden_vectors.is_file(), "golden vectors are checked in");

    let harness_dir = root.join("build/procgen-parity");
    std::fs::create_dir_all(&harness_dir).expect("create parity harness directory");
    let project = format!(
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    <TargetFramework>net8.0</TargetFramework>\n    <ImplicitUsings>enable</ImplicitUsings>\n    <Nullable>enable</Nullable>\n    <EnableDefaultCompileItems>false</EnableDefaultCompileItems>\n  </PropertyGroup>\n  <ItemGroup>\n    <Compile Include=\"{}\" />\n    <Compile Include=\"{}\" />\n    <Compile Include=\"{}\" />\n    <Compile Include=\"{}\" />\n  </ItemGroup>\n</Project>\n",
        sdk_source.display(),
        runtime_assets_source.display(),
        online_xr_source.display(),
        harness_source.display()
    );
    let project_path = harness_dir.join("ProcGenParity.csproj");
    std::fs::write(&project_path, project).expect("write parity harness project");

    let output = Command::new("dotnet")
        .arg("run")
        .arg("--project")
        .arg(&project_path)
        .arg("--")
        .arg(&golden_vectors)
        .output()
        .expect("run procgen parity harness");
    assert_success(&output, "procgen parity harness");

    let _ = std::fs::remove_dir_all(root);
}
