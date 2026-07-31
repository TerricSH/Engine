use super::*;

pub(super) fn validate_release_version(version: &str) -> Result<(), String> {
    let bytes = version.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return Err("release version must contain 1..=64 ASCII characters".into());
    }
    if !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "invalid release version '{version}'; use an ASCII letter or digit first, followed by letters, digits, '.', '_' or '-'"
        ));
    }
    Ok(())
}

pub(super) fn resolve_output_directory(
    project_root: &Path,
    requested: &Path,
    label: &str,
    require_project_local: bool,
) -> Result<PathBuf, String> {
    if requested.as_os_str().is_empty() {
        return Err(format!("{label} must not be empty"));
    }
    if requested
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!("{label} must not contain '..' traversal"));
    }
    let requested_path = if requested.is_absolute() {
        requested.to_path_buf()
    } else {
        project_root.join(requested)
    };
    let comparable =
        portable_windows_path(resolve_through_existing_ancestor(&requested_path, label)?);
    let project_root =
        portable_windows_path(std::fs::canonicalize(project_root).map_err(|error| {
            format!(
                "could not resolve project root {}: {error}",
                project_root.display()
            )
        })?);
    if comparable.parent().is_none() || comparable == project_root {
        return Err(format!(
            "{label} must be a dedicated directory, not a filesystem or project root"
        ));
    }
    if require_project_local && !comparable.starts_with(&project_root) {
        return Err(format!(
            "{label} must remain inside the project workspace for an installed engine: {}",
            requested.display()
        ));
    }
    if requested_path.is_file() {
        return Err(format!(
            "{label} points to a regular file: {}",
            requested_path.display()
        ));
    }
    Ok(comparable)
}

pub(super) fn validate_installed_package_output(
    project: &GameProject,
    output_root: &Path,
    version: &str,
) -> Result<(), String> {
    let output_root = portable_windows_path(resolve_through_existing_ancestor(
        output_root,
        "package output root",
    )?);
    let release_root = portable_windows_path(resolve_through_existing_ancestor(
        &output_root.join(version),
        "package release directory",
    )?);
    let mut protected_directories = vec![
        ("cooked_assets", project.cooked_assets.clone()),
        ("managed script SDK", project.root.join("build/script-sdk")),
        (
            "managed script host",
            project.root.join("build/script-host"),
        ),
    ];
    if let Some(script_output) = project.script_assembly.as_deref().and_then(Path::parent) {
        protected_directories.push(("script_assembly output", script_output.to_path_buf()));
    }

    for (protected_label, protected_path) in protected_directories {
        let protected_path = portable_windows_path(resolve_through_existing_ancestor(
            &protected_path,
            protected_label,
        )?);
        for (candidate_label, candidate) in [
            ("package output root", output_root.as_path()),
            ("package release directory", release_root.as_path()),
        ] {
            if paths_overlap(candidate, &protected_path) {
                return Err(format!(
                    "{candidate_label} {} overlaps the project-owned {protected_label} directory {}; choose a dedicated package output directory",
                    candidate.display(),
                    protected_path.display()
                ));
            }
        }
    }
    Ok(())
}

fn paths_overlap(left: &Path, right: &Path) -> bool {
    left == right || left.starts_with(right) || right.starts_with(left)
}

fn resolve_through_existing_ancestor(path: &Path, label: &str) -> Result<PathBuf, String> {
    let mut existing = path.to_path_buf();
    let mut missing = Vec::<OsString>::new();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    format!(
                        "{label} has no existing filesystem ancestor: {}",
                        path.display()
                    )
                })?;
                missing.push(name.to_os_string());
                existing = existing
                    .parent()
                    .ok_or_else(|| {
                        format!(
                            "{label} has no existing filesystem ancestor: {}",
                            path.display()
                        )
                    })?
                    .to_path_buf();
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect {label} ancestor {}: {error}",
                    existing.display()
                ))
            }
        }
    }
    let mut resolved = std::fs::canonicalize(&existing).map_err(|error| {
        format!(
            "could not resolve {label} ancestor {}: {error}",
            existing.display()
        )
    })?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

#[cfg(windows)]
pub(super) fn system_windows_executable(relative: &str) -> Result<PathBuf, String> {
    let system_root = std::env::var_os("SystemRoot")
        .filter(|value| !value.is_empty())
        .ok_or_else(|| "SystemRoot is not configured".to_string())?;
    canonical_file(
        PathBuf::from(system_root).join("System32").join(relative),
        relative,
    )
}

#[cfg(windows)]
pub(super) fn system_powershell_executable() -> Result<PathBuf, String> {
    system_windows_executable(r"WindowsPowerShell\v1.0\powershell.exe")
}

#[cfg(not(windows))]
pub(super) fn system_powershell_executable() -> Result<PathBuf, String> {
    Ok(PathBuf::from("powershell.exe"))
}

pub(super) fn canonical_directory(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "{label} is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(portable_windows_path(canonical))
}

pub(super) fn canonical_file(path: PathBuf, label: &str) -> Result<PathBuf, String> {
    let canonical = std::fs::canonicalize(&path)
        .map_err(|error| format!("could not resolve {label} {}: {error}", path.display()))?;
    if !canonical.is_file() {
        return Err(format!(
            "{label} is not a regular file: {}",
            canonical.display()
        ));
    }
    Ok(portable_windows_path(canonical))
}

#[cfg(windows)]
fn portable_windows_path(path: PathBuf) -> PathBuf {
    let display = path.to_string_lossy();
    if let Some(unc) = display.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(ordinary) = display.strip_prefix(r"\\?\") {
        PathBuf::from(ordinary)
    } else {
        path
    }
}

#[cfg(not(windows))]
fn portable_windows_path(path: PathBuf) -> PathBuf {
    path
}

pub(super) fn read_json(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("could not read result {}: {error}", path.display()))?;
    serde_json::from_slice(&bytes)
        .map_err(|error| format!("result is not valid JSON {}: {error}", path.display()))
}

pub(super) fn require_json_string(
    value: &Value,
    field: &str,
    expected: &str,
) -> Result<(), String> {
    let actual = value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("result field '{field}' is missing or is not a string"))?;
    if actual != expected {
        return Err(format!(
            "result field '{field}' is '{actual}', expected '{expected}'"
        ));
    }
    Ok(())
}

pub(super) fn required_string(value: &Value, field: &str) -> Result<String, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| format!("result field '{field}' is missing or is not a string"))
}

pub(super) fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("result field '{field}' is missing or is not an unsigned integer"))
}

pub(super) fn verify_checksum_sidecar(path: &Path) -> Result<String, String> {
    if !path.is_file() {
        return Err(format!("package artifact is missing: {}", path.display()));
    }
    let mut sidecar = path.as_os_str().to_os_string();
    sidecar.push(".sha256");
    let sidecar = PathBuf::from(sidecar);
    let contents = std::fs::read_to_string(&sidecar)
        .map_err(|error| format!("could not read checksum {}: {error}", sidecar.display()))?;
    let mut fields = contents.split_whitespace();
    let expected_hash = fields
        .next()
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or_else(|| format!("checksum file is invalid: {}", sidecar.display()))?
        .to_ascii_lowercase();
    let expected_name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        format!(
            "package artifact has no portable file name: {}",
            path.display()
        )
    })?;
    let sidecar_name = fields
        .next()
        .ok_or_else(|| format!("checksum file has no artifact name: {}", sidecar.display()))?;
    if sidecar_name != expected_name || fields.next().is_some() {
        return Err(format!(
            "checksum file does not name exactly '{expected_name}': {}",
            sidecar.display()
        ));
    }

    let mut file = File::open(path)
        .map_err(|error| format!("could not read package {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("could not hash package {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    let actual_hash = format!("{:x}", hasher.finalize());
    if actual_hash != expected_hash {
        return Err(format!(
            "package checksum mismatch for {}: expected {expected_hash}, got {actual_hash}",
            path.display()
        ));
    }
    Ok(actual_hash)
}
