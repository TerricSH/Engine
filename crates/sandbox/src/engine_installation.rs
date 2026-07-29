//! Installed-engine layout discovery.
//!
//! Game projects are deliberately independent from the engine source tree.
//! A distributed editor carries a small manifest beside its binaries; the
//! manifest identifies the prebuilt runtime, asset cooker, managed SDK, script
//! host, and packaging tool that may be copied into a project or release.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const ENGINE_INSTALLATION_FILE_NAME: &str = "engine.installation.json";
pub(crate) const ENGINE_INSTALLATION_SCHEMA: &str = "EngineInstallation-v0";
pub(crate) const ENGINE_INSTALL_ROOT_ENV: &str = "ENGINE_INSTALL_ROOT";
pub(crate) const ENGINE_SOURCE_ROOT_ENV: &str = "ENGINE_SOURCE_ROOT";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct EngineInstallationManifest {
    pub(crate) schema: String,
    pub(crate) engine_version: String,
    pub(crate) editor: PathBuf,
    pub(crate) windows_runtime: PathBuf,
    pub(crate) windows_symbols: PathBuf,
    pub(crate) asset_cooker: PathBuf,
    pub(crate) package_script: PathBuf,
    pub(crate) managed_sdk: PathBuf,
    pub(crate) script_host: PathBuf,
    pub(crate) notices: PathBuf,
    pub(crate) script_api: String,
    pub(crate) script_api_version: String,
    pub(crate) script_api_sha256: String,
    pub(crate) source_commit: String,
    pub(crate) source_date_epoch: i64,
    pub(crate) rustc: String,
    pub(crate) files: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // Individual installed capabilities are feature-gated per binary build.
pub(crate) struct EngineInstallation {
    pub(crate) root: PathBuf,
    pub(crate) manifest: EngineInstallationManifest,
    pub(crate) editor: PathBuf,
    pub(crate) windows_runtime: PathBuf,
    pub(crate) package_script: PathBuf,
    pub(crate) managed_sdk: PathBuf,
    pub(crate) script_host: PathBuf,
}

impl EngineInstallation {
    pub(crate) fn load(root: impl AsRef<Path>) -> Result<Self, String> {
        let requested_root = root.as_ref();
        let root = std::fs::canonicalize(requested_root).map_err(|error| {
            format!(
                "could not resolve engine installation root {}: {error}",
                requested_root.display()
            )
        })?;
        if !root.is_dir() {
            return Err(format!(
                "engine installation root is not a directory: {}",
                root.display()
            ));
        }

        let manifest_path = root.join(ENGINE_INSTALLATION_FILE_NAME);
        let json = std::fs::read_to_string(&manifest_path).map_err(|error| {
            format!(
                "could not read engine installation manifest {}: {error}",
                manifest_path.display()
            )
        })?;
        let manifest: EngineInstallationManifest =
            serde_json::from_str(&json).map_err(|error| {
                format!(
                    "could not parse engine installation manifest {}: {error}",
                    manifest_path.display()
                )
            })?;
        if manifest.schema != ENGINE_INSTALLATION_SCHEMA {
            return Err(format!(
                "unsupported engine installation schema '{}'; expected '{}'",
                manifest.schema, ENGINE_INSTALLATION_SCHEMA
            ));
        }
        if manifest.engine_version.trim().is_empty() {
            return Err("engine installation version must not be empty".into());
        }
        // ZIP timestamps are constrained to the DOS date range used by the
        // deterministic packager.
        if !(315_532_800..=4_354_819_199).contains(&manifest.source_date_epoch) {
            return Err(
                "engine installation source_date_epoch must be between 1980-01-01 and 2107-12-31"
                    .into(),
            );
        }
        if manifest.script_api.trim().is_empty()
            || manifest.script_api_version.trim().is_empty()
            || manifest.script_api_sha256.len() != 64
            || !manifest
                .script_api_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("engine installation managed SDK metadata is incomplete or invalid".into());
        }
        verify_manifest_hashes(&root, &manifest.files)?;

        let editor = resolve_file(&root, "editor", &manifest.editor)?;
        let windows_runtime = resolve_file(&root, "windows_runtime", &manifest.windows_runtime)?;
        resolve_file(&root, "windows_symbols", &manifest.windows_symbols)?;
        resolve_file(&root, "asset_cooker", &manifest.asset_cooker)?;
        let package_script = resolve_file(&root, "package_script", &manifest.package_script)?;
        let managed_sdk = resolve_file(&root, "managed_sdk", &manifest.managed_sdk)?;
        let script_host = resolve_directory(&root, "script_host", &manifest.script_host)?;
        resolve_file(&root, "notices", &manifest.notices)?;
        for (field, relative) in [
            ("editor", manifest.editor.as_path()),
            ("windows_runtime", manifest.windows_runtime.as_path()),
            ("asset_cooker", manifest.asset_cooker.as_path()),
            ("package_script", manifest.package_script.as_path()),
            ("managed_sdk", manifest.managed_sdk.as_path()),
            ("notices", manifest.notices.as_path()),
        ] {
            require_hashed_path(&manifest.files, field, relative)?;
        }
        require_hashed_path(
            &manifest.files,
            "windows_symbols",
            &manifest.windows_symbols,
        )?;
        require_hashed_directory_files(&manifest.files, &root, &manifest.script_host)?;

        Ok(Self {
            root,
            manifest,
            editor,
            windows_runtime,
            package_script,
            managed_sdk,
            script_host,
        })
    }

    /// Discover an installed layout from an explicit environment override or
    /// by walking upward from the running executable.
    ///
    /// Absence is not an error: source-tree development deliberately falls
    /// back to the repository toolchain. An explicit override is fail-closed.
    pub(crate) fn discover_from_current_executable() -> Result<Option<Self>, String> {
        let executable = std::env::current_exe()
            .map_err(|error| format!("could not resolve the running executable: {error}"))?;
        let override_root = std::env::var_os(ENGINE_INSTALL_ROOT_ENV)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from);
        Self::discover(&executable, override_root.as_deref())
    }

    pub(crate) fn discover(
        executable: &Path,
        override_root: Option<&Path>,
    ) -> Result<Option<Self>, String> {
        if let Some(root) = override_root {
            let installation = Self::load(root).map_err(|error| {
                format!("{ENGINE_INSTALL_ROOT_ENV} points to an invalid installation: {error}")
            })?;
            installation
                .validate_process_executable(executable)
                .map_err(|error| {
                    format!("{ENGINE_INSTALL_ROOT_ENV} does not match this engine process: {error}")
                })?;
            return Ok(Some(installation));
        }

        let executable = std::fs::canonicalize(executable).map_err(|error| {
            format!(
                "could not resolve engine executable {}: {error}",
                executable.display()
            )
        })?;
        let mut visited = BTreeSet::<OsString>::new();
        for candidate in executable.ancestors().skip(1).take(6) {
            if !visited.insert(candidate.as_os_str().to_os_string()) {
                continue;
            }
            if candidate.join(ENGINE_INSTALLATION_FILE_NAME).is_file() {
                let installation = Self::load(candidate)?;
                installation.validate_process_executable(&executable)?;
                return Ok(Some(installation));
            }
        }
        Ok(None)
    }

    fn validate_process_executable(&self, executable: &Path) -> Result<(), String> {
        let executable = std::fs::canonicalize(executable).map_err(|error| {
            format!(
                "could not resolve engine process executable {}: {error}",
                executable.display()
            )
        })?;
        if executable != self.editor && executable != self.windows_runtime {
            return Err(format!(
                "{} is neither the installed editor {} nor installed runtime {}",
                executable.display(),
                self.editor.display(),
                self.windows_runtime.display()
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_managed_sdk_contract(
        &self,
        script_api: &str,
        script_api_version: &str,
        script_api_sha256: &str,
    ) -> Result<(), String> {
        let installed = &self.manifest;
        if installed.script_api != script_api
            || installed.script_api_version != script_api_version
            || !installed
                .script_api_sha256
                .eq_ignore_ascii_case(script_api_sha256)
        {
            return Err(format!(
                "installed EngineGameplay SDK does not match this engine process: \
                 installed {} {} {}, process {} {} {}",
                installed.script_api,
                installed.script_api_version,
                installed.script_api_sha256,
                script_api,
                script_api_version,
                script_api_sha256
            ));
        }
        Ok(())
    }
}

/// Resolve the source-tree toolchain used only while developing the engine.
///
/// A copied executable without an installation manifest must not silently
/// reach back into the build machine's source checkout. The implicit fallback
/// is therefore accepted only for binaries running below `<workspace>/target`.
/// Custom development target directories must opt in with
/// `ENGINE_SOURCE_ROOT`.
pub(crate) fn development_source_root() -> Result<PathBuf, String> {
    if let Some(explicit) =
        std::env::var_os(ENGINE_SOURCE_ROOT_ENV).filter(|value| !value.is_empty())
    {
        return validate_source_root(&PathBuf::from(explicit)).map_err(|error| {
            format!("{ENGINE_SOURCE_ROOT_ENV} points to an invalid source tree: {error}")
        });
    }

    let manifest_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let compiled_root = manifest_directory
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| "sandbox crate is not inside an engine workspace".to_string())?;
    let root = validate_source_root(compiled_root)?;
    let executable = std::fs::canonicalize(
        std::env::current_exe()
            .map_err(|error| format!("could not resolve the running executable: {error}"))?,
    )
    .map_err(|error| format!("could not resolve the running executable: {error}"))?;
    let target = root.join("target");
    if !executable.starts_with(&target) {
        return Err(format!(
            "no engine installation manifest was found and {} is outside the development target \
             directory {}; install the engine or set {ENGINE_SOURCE_ROOT_ENV} explicitly",
            executable.display(),
            target.display()
        ));
    }
    Ok(root)
}

fn validate_source_root(root: &Path) -> Result<PathBuf, String> {
    let root = std::fs::canonicalize(root)
        .map_err(|error| format!("could not resolve source root {}: {error}", root.display()))?;
    if !root.join("Cargo.toml").is_file() || !root.join("crates/sandbox/Cargo.toml").is_file() {
        return Err(format!(
            "{} is not an engine source workspace",
            root.display()
        ));
    }
    Ok(root)
}

fn validate_relative_install_path(field: &str, path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty() || path.is_absolute() {
        return Err(format!(
            "engine installation field '{field}' must be a non-empty relative path"
        ));
    }
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) {
        return Err(format!(
            "engine installation field '{field}' may not escape the installation root: {}",
            path.display()
        ));
    }
    Ok(())
}

fn resolve_file(root: &Path, field: &str, relative: &Path) -> Result<PathBuf, String> {
    validate_relative_install_path(field, relative)?;
    let requested = root.join(relative);
    let resolved = std::fs::canonicalize(&requested).map_err(|error| {
        format!(
            "engine installation file '{field}' was not found at {}: {error}",
            requested.display()
        )
    })?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(format!(
            "engine installation field '{field}' is not a file inside {}: {}",
            root.display(),
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn resolve_directory(root: &Path, field: &str, relative: &Path) -> Result<PathBuf, String> {
    validate_relative_install_path(field, relative)?;
    let requested = root.join(relative);
    let resolved = std::fs::canonicalize(&requested).map_err(|error| {
        format!(
            "engine installation directory '{field}' was not found at {}: {error}",
            requested.display()
        )
    })?;
    if !resolved.starts_with(root) || !resolved.is_dir() {
        return Err(format!(
            "engine installation field '{field}' is not a directory inside {}: {}",
            root.display(),
            resolved.display()
        ));
    }
    Ok(resolved)
}

fn portable_relative_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|error| {
        format!(
            "could not hash installation file {}: {error}",
            path.display()
        )
    })?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            format!(
                "could not hash installation file {}: {error}",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn verify_manifest_hashes(root: &Path, files: &BTreeMap<String, String>) -> Result<(), String> {
    if files.is_empty() {
        return Err("engine installation manifest contains no file hashes".into());
    }
    for (relative_text, expected) in files {
        if !valid_sha256(expected) {
            return Err(format!(
                "engine installation hash for '{relative_text}' is not SHA-256"
            ));
        }
        let relative = Path::new(relative_text);
        validate_relative_install_path("files", relative)?;
        let resolved = resolve_file(root, "files", relative)?;
        let actual = sha256_file(&resolved)?;
        if !actual.eq_ignore_ascii_case(expected) {
            return Err(format!(
                "engine installation file hash mismatch for '{}': expected {}, got {}",
                relative_text, expected, actual
            ));
        }
    }
    Ok(())
}

fn require_hashed_path(
    files: &BTreeMap<String, String>,
    field: &str,
    relative: &Path,
) -> Result<(), String> {
    let portable = portable_relative_path(relative);
    if !files.contains_key(&portable) {
        return Err(format!(
            "engine installation field '{field}' is not covered by files SHA-256: {portable}"
        ));
    }
    Ok(())
}

fn require_hashed_directory_files(
    files: &BTreeMap<String, String>,
    root: &Path,
    relative_directory: &Path,
) -> Result<(), String> {
    let directory = resolve_directory(root, "script_host", relative_directory)?;
    let mut found_file = false;
    for entry in std::fs::read_dir(&directory)
        .map_err(|error| format!("could not enumerate {}: {error}", directory.display()))?
    {
        let entry = entry
            .map_err(|error| format!("could not enumerate {}: {error}", directory.display()))?;
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "could not inspect installation script host entry {}: {error}",
                entry.path().display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "engine installation script_host contains a non-file entry: {}",
                entry.path().display()
            ));
        }
        found_file = true;
        let entry_path = entry.path();
        let relative = entry_path.strip_prefix(root).map_err(|_| {
            format!(
                "engine installation script host escaped its root: {}",
                entry_path.display()
            )
        })?;
        require_hashed_path(files, "script_host", relative)?;
    }
    if !found_file {
        return Err("engine installation script_host contains no files".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn installation_fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("engine");
        for folder in [
            "bin",
            "runtime/windows-x86_64",
            "tools",
            "sdk",
            "sdk/script-host",
        ] {
            std::fs::create_dir_all(root.join(folder)).unwrap();
        }
        for file in [
            "bin/EngineEditor.exe",
            "runtime/windows-x86_64/GameRuntime.exe",
            "runtime/windows-x86_64/GameRuntime.pdb",
            "tools/asset-cook.exe",
            "tools/package-windows.ps1",
            "sdk/EngineGameplay.dll",
            "sdk/script-host/EngineScriptHost.exe",
            "THIRD_PARTY_NOTICES.txt",
        ] {
            std::fs::write(root.join(file), file.as_bytes()).unwrap();
        }
        let files = [
            "bin/EngineEditor.exe",
            "runtime/windows-x86_64/GameRuntime.exe",
            "runtime/windows-x86_64/GameRuntime.pdb",
            "tools/asset-cook.exe",
            "tools/package-windows.ps1",
            "sdk/EngineGameplay.dll",
            "sdk/script-host/EngineScriptHost.exe",
            "THIRD_PARTY_NOTICES.txt",
        ]
        .into_iter()
        .map(|path| {
            (
                path.to_string(),
                sha256_file(&root.join(path)).expect("fixture hash"),
            )
        })
        .collect();
        let manifest = EngineInstallationManifest {
            schema: ENGINE_INSTALLATION_SCHEMA.into(),
            engine_version: "0.1.0".into(),
            editor: "bin/EngineEditor.exe".into(),
            windows_runtime: "runtime/windows-x86_64/GameRuntime.exe".into(),
            windows_symbols: "runtime/windows-x86_64/GameRuntime.pdb".into(),
            asset_cooker: "tools/asset-cook.exe".into(),
            package_script: "tools/package-windows.ps1".into(),
            managed_sdk: "sdk/EngineGameplay.dll".into(),
            script_host: "sdk/script-host".into(),
            notices: "THIRD_PARTY_NOTICES.txt".into(),
            script_api: "ScriptAPI-v0".into(),
            script_api_version: "0.15.0".into(),
            script_api_sha256: "a".repeat(64),
            source_commit: "fixture".into(),
            source_date_epoch: 1_700_000_000,
            rustc: "rustc fixture".into(),
            files,
        };
        std::fs::write(
            root.join(ENGINE_INSTALLATION_FILE_NAME),
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let editor = root.join("bin/EngineEditor.exe");
        (directory, root, editor)
    }

    #[test]
    fn discovers_a_manifest_above_a_nested_executable() {
        let (_directory, root, editor) = installation_fixture();
        let installation = EngineInstallation::discover(&editor, None)
            .unwrap()
            .expect("installed layout");
        assert_eq!(installation.root, std::fs::canonicalize(root).unwrap());
        assert!(installation.managed_sdk.ends_with("sdk/EngineGameplay.dll"));
        assert!(installation.script_host.is_dir());
    }

    #[test]
    fn discovers_the_installed_runtime_from_its_deeper_layout() {
        let (_directory, root, _editor) = installation_fixture();
        let runtime = root.join("runtime/windows-x86_64/GameRuntime.exe");
        let installation = EngineInstallation::discover(&runtime, None)
            .unwrap()
            .expect("runtime installation");
        assert_eq!(
            installation.windows_runtime,
            std::fs::canonicalize(runtime).unwrap()
        );
    }

    #[test]
    fn automatic_discovery_rejects_an_unregistered_process_inside_the_installation() {
        let (_directory, root, _editor) = installation_fixture();
        let asset_cooker = root.join("tools/asset-cook.exe");
        let error = EngineInstallation::discover(&asset_cooker, None).unwrap_err();
        assert!(error.contains("neither the installed editor"));
    }

    #[test]
    fn executable_without_an_installation_manifest_uses_the_development_fallback() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("sandbox.exe");
        std::fs::write(&executable, b"standalone").unwrap();
        assert!(EngineInstallation::discover(&executable, None)
            .unwrap()
            .is_none());
    }

    #[test]
    fn explicit_installation_override_fails_closed() {
        let (directory, _root, editor) = installation_fixture();
        let missing = directory.path().join("missing");
        let error = EngineInstallation::discover(&editor, Some(&missing)).unwrap_err();
        assert!(error.contains(ENGINE_INSTALL_ROOT_ENV));
    }

    #[test]
    fn explicit_installation_override_rejects_a_foreign_engine_process() {
        let (directory, root, _editor) = installation_fixture();
        let foreign = directory.path().join("foreign-editor.exe");
        std::fs::write(&foreign, b"foreign").unwrap();
        let error = EngineInstallation::discover(&foreign, Some(&root)).unwrap_err();
        assert!(error.contains("does not match this engine process"));
    }

    #[test]
    fn manifest_paths_cannot_escape_the_installation() {
        let (_directory, root, _editor) = installation_fixture();
        let manifest_path = root.join(ENGINE_INSTALLATION_FILE_NAME);
        let mut manifest: EngineInstallationManifest =
            serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
        manifest.managed_sdk = "../outside.dll".into();
        std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let error = EngineInstallation::load(&root).unwrap_err();
        assert!(error.contains("may not escape"));
    }

    #[test]
    fn managed_sdk_contract_must_match_the_running_engine() {
        let (_directory, root, _editor) = installation_fixture();
        let installation = EngineInstallation::load(root).unwrap();
        installation
            .validate_managed_sdk_contract("ScriptAPI-v0", "0.15.0", &"a".repeat(64))
            .unwrap();
        assert!(installation
            .validate_managed_sdk_contract("ScriptAPI-v0", "0.16.0", &"a".repeat(64))
            .is_err());
    }

    #[test]
    fn load_rejects_a_tampered_hashed_artifact() {
        let (_directory, root, _editor) = installation_fixture();
        std::fs::write(root.join("sdk/EngineGameplay.dll"), b"tampered-content").unwrap();
        let error = EngineInstallation::load(root).unwrap_err();
        assert!(error.contains("file hash mismatch"));
        assert!(error.contains("sdk/EngineGameplay.dll"));
    }

    #[test]
    fn load_rejects_an_unregistered_script_host_file() {
        let (_directory, root, _editor) = installation_fixture();
        std::fs::write(root.join("sdk/script-host/Injected.dll"), b"unregistered").unwrap();
        let error = EngineInstallation::load(root).unwrap_err();
        assert!(error.contains("script_host"));
        assert!(error.contains("Injected.dll"));
    }
}
