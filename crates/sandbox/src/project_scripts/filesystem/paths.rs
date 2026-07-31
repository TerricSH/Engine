use super::*;

pub(crate) fn sibling_with_suffix(path: &Path, suffix: &str) -> Result<PathBuf, String> {
    let name = path
        .file_name()
        .ok_or_else(|| format!("path has no final component: {}", path.display()))?;
    let mut next_name = OsString::from(name);
    next_name.push(suffix);
    Ok(path.with_file_name(next_name))
}

pub(crate) fn reset_owned_directory(project_root: &Path, directory: &Path) -> Result<(), String> {
    ensure_inside_project(project_root, directory, "generated directory")?;
    if directory.exists() {
        std::fs::remove_dir_all(directory)
            .map_err(|error| format!("could not clear {}: {error}", directory.display()))?;
    }
    std::fs::create_dir_all(directory)
        .map_err(|error| format!("could not create {}: {error}", directory.display()))
}

pub(crate) fn replace_owned_directory(
    project_root: &Path,
    next: &Path,
    final_path: &Path,
) -> Result<(), String> {
    ensure_inside_project(project_root, next, "generated next directory")?;
    ensure_inside_project(project_root, final_path, "generated output directory")?;
    let backup = sibling_with_suffix(final_path, ".previous")?;
    ensure_inside_project(project_root, &backup, "generated backup directory")?;
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .map_err(|error| format!("could not clear {}: {error}", backup.display()))?;
    }
    if final_path.exists() {
        std::fs::rename(final_path, &backup).map_err(|error| {
            format!(
                "could not preserve previous generated output {}: {error}",
                final_path.display()
            )
        })?;
    }
    if let Err(error) = std::fs::rename(next, final_path) {
        if backup.exists() {
            let _ = std::fs::rename(&backup, final_path);
        }
        return Err(format!(
            "could not activate generated output {}: {error}",
            final_path.display()
        ));
    }
    if backup.exists() {
        std::fs::remove_dir_all(&backup)
            .map_err(|error| format!("could not remove {}: {error}", backup.display()))?;
    }
    Ok(())
}

pub(crate) fn ensure_inside_project(root: &Path, path: &Path, field: &str) -> Result<(), String> {
    if path.components().any(|component| {
        matches!(
            component,
            Component::ParentDir | Component::RootDir | Component::Prefix(_)
        )
    }) && !path.is_absolute()
    {
        return Err(format!(
            "{field} contains unsafe path traversal: {}",
            path.display()
        ));
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    if absolute
        .components()
        .any(|component| matches!(component, Component::ParentDir))
    {
        return Err(format!(
            "{field} contains unsafe path traversal: {}",
            path.display()
        ));
    }
    let resolved_root = std::fs::canonicalize(root)
        .map_err(|error| format!("could not resolve project root {}: {error}", root.display()))?;
    let resolved = resolve_through_existing_ancestor(&absolute, field)?;
    if !resolved.starts_with(&resolved_root) || resolved == resolved_root {
        return Err(format!(
            "{field} must remain inside the project root: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn resolve_through_existing_ancestor(
    path: &Path,
    field: &str,
) -> Result<PathBuf, String> {
    let mut existing = path.to_path_buf();
    let mut missing = Vec::<OsString>::new();
    loop {
        match std::fs::symlink_metadata(&existing) {
            Ok(_) => break,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = existing.file_name().ok_or_else(|| {
                    format!(
                        "{field} has no existing filesystem ancestor: {}",
                        path.display()
                    )
                })?;
                missing.push(name.to_os_string());
                existing = existing
                    .parent()
                    .ok_or_else(|| {
                        format!(
                            "{field} has no existing filesystem ancestor: {}",
                            path.display()
                        )
                    })?
                    .to_path_buf();
            }
            Err(error) => {
                return Err(format!(
                    "could not inspect {field} ancestor {}: {error}",
                    existing.display()
                ))
            }
        }
    }
    let mut resolved = std::fs::canonicalize(&existing).map_err(|error| {
        format!(
            "could not resolve {field} ancestor {}: {error}",
            existing.display()
        )
    })?;
    for component in missing.into_iter().rev() {
        resolved.push(component);
    }
    Ok(resolved)
}

pub(crate) fn write_file(path: &Path, content: &str) -> Result<(), String> {
    std::fs::write(path, content)
        .map_err(|error| format!("could not write {}: {error}", path.display()))
}

pub(crate) fn report_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
