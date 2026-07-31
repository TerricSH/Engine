use super::*;

pub(super) fn load_project(project_path: &Path) -> Result<GameProject, String> {
    GameProject::load(project_path).map_err(|error| error.to_string())
}

pub(super) fn lock_asset_operations() -> Result<MutexGuard<'static, ()>, String> {
    ASSET_OPERATION_MUTEX
        .lock()
        .map_err(|_| "asset operation lock was poisoned by a prior panic".to_string())
}

pub(super) fn cooked_path(project: &GameProject, asset_id: &AssetId) -> PathBuf {
    project
        .cooked_assets
        .join(format!("{}.cooked", asset_id.id))
}

pub(super) fn validate_asset_id(asset_id: &AssetId) -> Result<(), String> {
    engine_asset::validate_asset_id(asset_id).map_err(|error| error.to_string())?;
    if asset_id.id.len() > 128 {
        return Err("asset id may not exceed 128 bytes".into());
    }
    if !asset_id
        .id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(format!(
            "asset id '{}' must use ASCII letters, digits, hyphens, underscores, or dots",
            asset_id.id
        ));
    }
    Ok(())
}

pub(super) fn normalize_relative_path(
    path: &Path,
    label: &str,
    allow_empty: bool,
) -> Result<PathBuf, String> {
    if path.as_os_str().is_empty() {
        return if allow_empty {
            Ok(PathBuf::new())
        } else {
            Err(format!("{label} may not be empty"))
        };
    }
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => {
                let component = component
                    .to_str()
                    .ok_or_else(|| format!("{label} contains non-UTF-8 text"))?;
                validate_portable_component(component, label)?;
                normalized.push(component);
            }
            Component::CurDir => {
                return Err(format!("{label} may not contain '.' path components"));
            }
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!("{label} must be a portable project-relative path"));
            }
        }
    }
    if normalized.as_os_str().is_empty() && !allow_empty {
        Err(format!("{label} may not be empty"))
    } else {
        Ok(normalized)
    }
}

fn validate_portable_component(component: &str, label: &str) -> Result<(), String> {
    if component.is_empty()
        || component.ends_with([' ', '.'])
        || component.chars().any(|character| {
            character.is_control()
                || matches!(
                    character,
                    '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                )
        })
    {
        return Err(format!(
            "{label} contains a non-portable path component '{component}'"
        ));
    }
    let stem = component
        .split('.')
        .next()
        .unwrap_or(component)
        .to_ascii_uppercase();
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.as_bytes()[3].is_ascii_digit()
            && stem.as_bytes()[3] != b'0')
    {
        return Err(format!(
            "{label} uses reserved portable file name '{component}'"
        ));
    }
    Ok(())
}

pub(super) fn manifest_path_string(path: &Path) -> Result<String, String> {
    let path = normalize_relative_path(path, "manifest source path", false)?;
    path.components()
        .map(|component| match component {
            Component::Normal(value) => value
                .to_str()
                .map(str::to_owned)
                .ok_or_else(|| "manifest source path contains non-UTF-8 text".to_string()),
            _ => Err("manifest source path is not normalized".into()),
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|parts| parts.join("/"))
}

pub(super) fn project_relative_string(
    project: &GameProject,
    path: &Path,
) -> Result<String, String> {
    let relative = path.strip_prefix(&project.root).map_err(|_| {
        format!(
            "path is outside project root and cannot be recorded in trash metadata: {}",
            path.display()
        )
    })?;
    manifest_path_string(relative)
}

pub(super) fn portable_path_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

pub(super) fn ensure_existing_folder(root: &Path, relative: &Path) -> Result<(), String> {
    if relative.as_os_str().is_empty() {
        return Ok(());
    }
    let path = root.join(relative);
    if !path.is_dir() {
        return Err(format!("asset folder does not exist: {}", path.display()));
    }
    reject_symlink(&path)
}

pub(super) fn ensure_parent_is_real_directory(root: &Path, relative: &Path) -> Result<(), String> {
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    ensure_existing_folder(root, parent)?;
    if let Some(case_match) = resolve_case_insensitive(root, parent)? {
        let requested = root.join(parent);
        if case_match != requested {
            return Err(format!(
                "asset path parent differs only by case from existing folder: {}",
                case_match.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn ensure_destination_absent(root: &Path, relative: &Path) -> Result<(), String> {
    if let Some(conflict) = resolve_case_insensitive(root, relative)? {
        return Err(format!(
            "asset destination already exists or differs only by case: {}",
            conflict.display()
        ));
    }
    Ok(())
}

pub(super) fn resolve_case_insensitive(
    root: &Path,
    relative: &Path,
) -> Result<Option<PathBuf>, String> {
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(format!("path is not normalized: {}", relative.display()));
        };
        if !current.is_dir() {
            return Ok(None);
        }
        let requested = component.to_string_lossy();
        let mut found = None;
        for entry in std::fs::read_dir(&current)
            .map_err(|error| format!("could not enumerate {}: {error}", current.display()))?
        {
            let entry = entry
                .map_err(|error| format!("could not enumerate {}: {error}", current.display()))?;
            if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case(&requested))
            {
                found = Some(entry.path());
                break;
            }
        }
        let Some(path) = found else {
            return Ok(None);
        };
        current = path;
    }
    Ok(Some(current))
}

pub(super) fn reject_symlink(path: &Path) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() {
        Err(format!(
            "asset operations do not follow symbolic links: {}",
            path.display()
        ))
    } else {
        Ok(())
    }
}

pub(super) fn ensure_no_symlink_ancestors(root: &Path, path: &Path) -> Result<(), String> {
    let relative = path.strip_prefix(root).map_err(|_| {
        format!(
            "asset operation path escapes project root: {}",
            path.display()
        )
    })?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        if current.exists() {
            reject_symlink(&current)?;
        }
    }
    Ok(())
}

pub(super) fn copy_directory_tree(source: &Path, destination: &Path) -> Result<(), String> {
    reject_symlink(source)?;
    std::fs::create_dir(destination).map_err(|error| {
        format!(
            "could not create staged source directory {}: {error}",
            destination.display()
        )
    })?;
    for entry in std::fs::read_dir(source)
        .map_err(|error| format!("could not enumerate {}: {error}", source.display()))?
    {
        let entry =
            entry.map_err(|error| format!("could not enumerate {}: {error}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        let metadata = std::fs::symlink_metadata(&source_path)
            .map_err(|error| format!("could not inspect {}: {error}", source_path.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "asset operations do not copy symbolic links: {}",
                source_path.display()
            ));
        }
        if metadata.is_dir() {
            copy_directory_tree(&source_path, &destination_path)?;
        } else if metadata.is_file() {
            copy_file_create_new(&source_path, &destination_path)?;
        } else {
            return Err(format!(
                "unsupported source-tree entry: {}",
                source_path.display()
            ));
        }
    }
    Ok(())
}

pub(super) fn copy_file_create_new(source: &Path, destination: &Path) -> Result<(), String> {
    let bytes = std::fs::read(source).map_err(io_read(source))?;
    write_file_create_new(destination, &bytes)
}

pub(super) fn write_file_create_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            format!(
                "could not create {} without overwriting an existing file: {error}",
                path.display()
            )
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = std::fs::remove_file(path);
        return Err(format!("could not write {}: {error}", path.display()));
    }
    Ok(())
}

pub(super) fn serialize_manifest(manifest: &SourceManifest) -> Result<Vec<u8>, String> {
    let mut bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("could not serialize source manifest: {error}"))?;
    bytes.push(b'\n');
    Ok(bytes)
}

pub(super) fn map_staged_path(
    staged_root: &Path,
    live_root: &Path,
    path: &Path,
) -> Result<PathBuf, String> {
    path.strip_prefix(staged_root)
        .map(|relative| live_root.join(relative))
        .map_err(|_| format!("staged path escapes source workspace: {}", path.display()))
}

pub(super) fn io_read(path: &Path) -> impl FnOnce(std::io::Error) -> String + '_ {
    move |error| format!("could not read {}: {error}", path.display())
}

pub(super) fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}
