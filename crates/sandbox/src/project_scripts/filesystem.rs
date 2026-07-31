use super::*;

pub(crate) fn assembly_id_from_path(path: &Path) -> Result<String, String> {
    let id = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(str::trim)
        .filter(|stem| !stem.is_empty())
        .ok_or_else(|| {
            format!(
                "managed assembly path has no valid file stem: {}",
                path.display()
            )
        })?;
    if id.chars().any(char::is_whitespace) {
        return Err(format!(
            "managed assembly id may not contain whitespace: {id:?}"
        ));
    }
    Ok(id.to_string())
}

pub(crate) fn managed_dependencies(
    directory: &Path,
    main_assembly: &Path,
) -> Result<Vec<PathBuf>, String> {
    let main_name = main_assembly.file_name();
    let mut dependencies = std::fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "could not enumerate managed output {}: {error}",
                directory.display()
            )
        })?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("dll"))
                && path.file_name() != main_name
        })
        .collect::<Vec<_>>();
    dependencies.sort();
    Ok(dependencies)
}

#[cfg(feature = "subsystem-scripting-csharp")]
pub(crate) fn resolve_script_host(project: &GameProject) -> Result<PathBuf, String> {
    if let Some(override_path) = std::env::var_os("ENGINE_SCRIPT_HOST") {
        let path = PathBuf::from(override_path);
        if path.is_file() {
            return Ok(path);
        }
        return Err(format!(
            "ENGINE_SCRIPT_HOST does not name a file: {}",
            path.display()
        ));
    }

    let project_host = project
        .root
        .join("build/script-host")
        .join(host_executable_name());
    if project_host.is_file() {
        return Ok(project_host);
    }
    let packaged_host = std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(Path::to_path_buf))
        .map(|directory| directory.join("script-host").join(host_executable_name()));
    if let Some(path) = packaged_host.filter(|path| path.is_file()) {
        return Ok(path);
    }
    Err(format!(
        "C# script host is missing; run `sandbox project build-scripts {}` or set ENGINE_SCRIPT_HOST",
        project.root.display()
    ))
}

pub(crate) fn host_executable_name() -> &'static str {
    if cfg!(windows) {
        "EngineScriptHost.exe"
    } else {
        "EngineScriptHost"
    }
}

pub(crate) fn file_contents_equal(path: &Path, expected: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|contents| contents == expected)
}

pub(crate) fn copy_installed_file(
    source: &Path,
    destination: &Path,
    label: &str,
) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| {
        format!(
            "could not inspect installed {label} {}: {error}",
            source.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "installed {label} is not a regular file: {}",
            source.display()
        ));
    }
    std::fs::copy(source, destination).map_err(|error| {
        format!(
            "could not copy installed {label} from {} to {}: {error}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(())
}

pub(crate) fn copy_installed_directory_files(
    source: &Path,
    destination: &Path,
    label: &str,
) -> Result<(), String> {
    let mut entries = std::fs::read_dir(source)
        .map_err(|error| {
            format!(
                "could not enumerate installed {label} {}: {error}",
                source.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "could not inspect installed {label} {}: {error}",
                source.display()
            )
        })?;
    entries.sort_by_key(std::fs::DirEntry::file_name);
    if entries.is_empty() {
        return Err(format!(
            "installed {label} directory is empty: {}",
            source.display()
        ));
    }
    for entry in entries {
        let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
            format!(
                "could not inspect installed {label} entry {}: {error}",
                entry.path().display()
            )
        })?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(format!(
                "installed {label} contains a non-file entry: {}",
                entry.path().display()
            ));
        }
        copy_installed_file(&entry.path(), &destination.join(entry.file_name()), label)?;
    }
    Ok(())
}

pub(crate) fn regular_files_equal(expected: &Path, actual: &Path) -> Result<bool, String> {
    if !actual.is_file() {
        return Ok(false);
    }
    let expected_metadata = std::fs::symlink_metadata(expected).map_err(|error| {
        format!(
            "could not inspect managed tool file {}: {error}",
            expected.display()
        )
    })?;
    let actual_metadata = std::fs::symlink_metadata(actual).map_err(|error| {
        format!(
            "could not inspect project managed tool file {}: {error}",
            actual.display()
        )
    })?;
    if expected_metadata.file_type().is_symlink()
        || actual_metadata.file_type().is_symlink()
        || !expected_metadata.is_file()
        || !actual_metadata.is_file()
        || expected_metadata.len() != actual_metadata.len()
    {
        return Ok(false);
    }
    let expected_bytes = std::fs::read(expected).map_err(|error| {
        format!(
            "could not compare managed tool file {}: {error}",
            expected.display()
        )
    })?;
    let actual_bytes = std::fs::read(actual).map_err(|error| {
        format!(
            "could not compare managed tool file {}: {error}",
            actual.display()
        )
    })?;
    Ok(expected_bytes == actual_bytes)
}

pub(crate) fn directory_contains_only_regular_file_equal(
    directory: &Path,
    expected_name: &str,
    expected_file: &Path,
) -> Result<bool, String> {
    if !directory.is_dir() {
        return Ok(false);
    }
    let mut entries = std::fs::read_dir(directory)
        .map_err(|error| {
            format!(
                "could not enumerate managed tool directory {}: {error}",
                directory.display()
            )
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| {
            format!(
                "could not inspect managed tool directory {}: {error}",
                directory.display()
            )
        })?;
    if entries.len() != 1 {
        return Ok(false);
    }
    let entry = entries.pop().expect("one directory entry");
    if entry.file_name() != expected_name {
        return Ok(false);
    }
    regular_files_equal(expected_file, &entry.path())
}

pub(crate) fn directory_files_equal(expected: &Path, actual: &Path) -> Result<bool, String> {
    if !actual.is_dir() {
        return Ok(false);
    }
    let list_files = |directory: &Path| -> Result<Vec<(OsString, PathBuf)>, String> {
        let mut files = Vec::new();
        for entry in std::fs::read_dir(directory).map_err(|error| {
            format!(
                "could not enumerate managed tool directory {}: {error}",
                directory.display()
            )
        })? {
            let entry = entry.map_err(|error| {
                format!(
                    "could not enumerate managed tool directory {}: {error}",
                    directory.display()
                )
            })?;
            let metadata = std::fs::symlink_metadata(entry.path()).map_err(|error| {
                format!(
                    "could not inspect managed tool entry {}: {error}",
                    entry.path().display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Ok(Vec::new());
            }
            files.push((entry.file_name(), entry.path()));
        }
        files.sort_by(|left, right| left.0.cmp(&right.0));
        Ok(files)
    };
    let expected_files = list_files(expected)?;
    let actual_files = list_files(actual)?;
    if expected_files.is_empty()
        || expected_files.len() != actual_files.len()
        || expected_files
            .iter()
            .zip(&actual_files)
            .any(|(left, right)| left.0 != right.0)
    {
        return Ok(false);
    }
    for ((_, expected_path), (_, actual_path)) in expected_files.iter().zip(&actual_files) {
        let expected_bytes = std::fs::read(expected_path).map_err(|error| {
            format!(
                "could not compare managed tool file {}: {error}",
                expected_path.display()
            )
        })?;
        let actual_bytes = std::fs::read(actual_path).map_err(|error| {
            format!(
                "could not compare managed tool file {}: {error}",
                actual_path.display()
            )
        })?;
        if expected_bytes != actual_bytes {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn self_test_script_host(
    executable: &Path,
    working_directory: &Path,
) -> Result<(), String> {
    let output = Command::new(executable)
        .arg("--self-test")
        .current_dir(working_directory)
        .output()
        .map_err(|error| format!("could not launch C# script host self-test: {error}"))?;
    ensure_command_success("C# script host gameplay bridge self-test", output)
}

pub(crate) fn resolve_dotnet_executable() -> Result<PathBuf, String> {
    let executable_name = if cfg!(windows) {
        "dotnet.exe"
    } else {
        "dotnet"
    };
    let mut candidates = Vec::new();
    if let Some(host) = std::env::var_os("DOTNET_HOST_PATH").filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(host));
    }
    if let Some(root) = std::env::var_os("DOTNET_ROOT").filter(|value| !value.is_empty()) {
        candidates.push(PathBuf::from(root).join(executable_name));
    }
    if let Some(path) = std::env::var_os("PATH") {
        candidates.extend(
            std::env::split_paths(&path)
                .filter(|directory| directory.is_absolute())
                .map(|directory| directory.join(executable_name)),
        );
    }
    for candidate in candidates {
        if !candidate.is_absolute() || !candidate.is_file() {
            continue;
        }
        let resolved = std::fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "could not resolve .NET host {}: {error}",
                candidate.display()
            )
        })?;
        if resolved.is_file() {
            return Ok(resolved);
        }
    }
    Err(
        "could not locate an absolute dotnet executable via DOTNET_HOST_PATH, DOTNET_ROOT, or PATH"
            .into(),
    )
}

pub(crate) fn ensure_command_success(
    label: &str,
    output: std::process::Output,
) -> Result<(), String> {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{label} failed with {}:\n{}{}{}",
        output.status,
        stdout,
        if stdout.is_empty() || stderr.is_empty() {
            ""
        } else {
            "\n"
        },
        stderr
    ))
}

mod paths;
pub(crate) use paths::*;
