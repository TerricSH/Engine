use super::*;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::sync::MutexGuard;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct SceneTrashMetadata {
    pub(crate) schema: String,
    pub(crate) deleted_unix_nanos: u128,
    pub(crate) scene_id: String,
    pub(crate) scene_name: String,
    pub(crate) original_scene_path: String,
    pub(crate) original_startup_scene: String,
    pub(crate) was_startup: bool,
    pub(crate) replacement_startup: Option<String>,
}

pub(crate) struct ProjectSceneOperationGuard {
    _mutex_guard: MutexGuard<'static, ()>,
    lock_file: Option<File>,
    lock_path: PathBuf,
}

impl Drop for ProjectSceneOperationGuard {
    fn drop(&mut self) {
        self.lock_file.take();
        let _ = std::fs::remove_file(&self.lock_path);
    }
}

pub(crate) fn lock_project_scene_operations(
    project_path: &Path,
) -> Result<(ProjectSceneOperationGuard, GameProject), String> {
    let mutex_guard = SCENE_OPERATION_MUTEX
        .lock()
        .map_err(|_| "project scene operation lock was poisoned by a prior panic".to_string())?;
    let initial = GameProject::load(project_path).map_err(|error| error.to_string())?;
    let lock_directory = initial.root.join(".engine/locks");
    ensure_scene_directory_chain(&initial.root, &lock_directory, true)?;
    let lock_path = lock_directory.join("scene-operations.lock");
    let created_unix_nanos = unix_nanos()?;
    let mut lock_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&lock_path)
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "another project scene operation is active (or left a stale lock): {}",
                    lock_path.display()
                )
            } else {
                format!(
                    "could not acquire project scene operation lock {}: {error}",
                    lock_path.display()
                )
            }
        })?;
    let owner = format!(
        "pid={}\ncreated_unix_nanos={}\n",
        std::process::id(),
        created_unix_nanos
    );
    if let Err(error) = lock_file
        .write_all(owner.as_bytes())
        .and_then(|()| lock_file.sync_all())
    {
        drop(lock_file);
        let _ = std::fs::remove_file(&lock_path);
        return Err(format!(
            "could not initialize project scene operation lock {}: {error}",
            lock_path.display()
        ));
    }
    let guard = ProjectSceneOperationGuard {
        _mutex_guard: mutex_guard,
        lock_file: Some(lock_file),
        lock_path,
    };
    // Reload after acquiring the cross-process lock so no catalog snapshot
    // taken before another process committed is used for a mutation.
    let project = GameProject::load(&initial.root).map_err(|error| error.to_string())?;
    Ok((guard, project))
}

pub(crate) fn exact_scene_catalog_path<'a>(
    catalog: &'a BTreeMap<String, PathBuf>,
    scene_id: &str,
) -> Result<&'a PathBuf, String> {
    if let Some(path) = catalog.get(scene_id) {
        return Ok(path);
    }
    if let Some((actual, _)) = catalog
        .iter()
        .find(|(existing, _)| existing.eq_ignore_ascii_case(scene_id))
    {
        return Err(format!(
            "project scene ID is case-sensitive; requested '{scene_id}', catalog contains '{actual}'"
        ));
    }
    Err(format!(
        "unknown project scene '{scene_id}'; available scenes: {}",
        catalog.keys().cloned().collect::<Vec<_>>().join(", ")
    ))
}

pub(crate) fn validate_portable_scene_id(scene_id: &str) -> Result<(), String> {
    let valid = !scene_id.is_empty()
        && scene_id.len() <= 128
        && scene_id != "."
        && scene_id != ".."
        && scene_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    if !valid {
        return Err(format!(
            "scene ID '{scene_id}' must contain 1..=128 ASCII letters, digits, hyphens, underscores, or dots"
        ));
    }
    let portable_stem = scene_id
        .split('.')
        .next()
        .unwrap_or(scene_id)
        .to_ascii_uppercase();
    if matches!(portable_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (portable_stem.len() == 4
            && (portable_stem.starts_with("COM") || portable_stem.starts_with("LPT"))
            && portable_stem.as_bytes()[3].is_ascii_digit()
            && portable_stem.as_bytes()[3] != b'0')
    {
        return Err(format!(
            "scene ID '{scene_id}' is not portable because it uses a reserved file name"
        ));
    }
    Ok(())
}

pub(crate) fn portable_scene_subdirectory(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "scene folder must be a safe project-relative path: {}",
                path.display()
            ));
        };
        let component = component
            .to_str()
            .ok_or_else(|| "scene folder contains non-UTF-8 text".to_string())?;
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
                "scene folder contains a non-portable component '{component}'"
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
                "scene folder uses reserved portable name '{component}'"
            ));
        }
        normalized.push(component);
    }
    Ok(normalized)
}

pub(crate) fn ensure_scene_file_is_regular(project_root: &Path, path: &Path) -> Result<(), String> {
    ensure_no_scene_symlink_ancestors(project_root, path)?;
    let metadata = std::fs::symlink_metadata(path)
        .map_err(|error| format!("could not inspect scene file {}: {error}", path.display()))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(format!(
            "project scene path is not a regular file: {}",
            path.display()
        ));
    }
    Ok(())
}

pub(crate) fn ensure_no_scene_symlink_ancestors(
    project_root: &Path,
    path: &Path,
) -> Result<(), String> {
    let relative = path.strip_prefix(project_root).map_err(|_| {
        format!(
            "project scene operation path escapes project root: {}",
            path.display()
        )
    })?;
    let mut current = project_root.to_path_buf();
    for component in relative.components() {
        match component {
            Component::Normal(component) => current.push(component),
            Component::CurDir => continue,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project scene operation path is not normalized: {}",
                    path.display()
                ));
            }
        }
        if current.exists() {
            let metadata = std::fs::symlink_metadata(&current)
                .map_err(|error| format!("could not inspect {}: {error}", current.display()))?;
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "project scene operations do not follow symbolic links: {}",
                    current.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn ensure_scene_directory_chain(
    project_root: &Path,
    directory: &Path,
    create_missing: bool,
) -> Result<(), String> {
    let relative = directory.strip_prefix(project_root).map_err(|_| {
        format!(
            "project scene directory escapes project root: {}",
            directory.display()
        )
    })?;
    let mut current = project_root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            if matches!(component, Component::CurDir) {
                continue;
            }
            return Err(format!(
                "project scene directory is not normalized: {}",
                directory.display()
            ));
        };
        let requested = component
            .to_str()
            .ok_or_else(|| "project scene directory contains non-UTF-8 text".to_string())?;
        if let Some(existing) = find_case_insensitive_entry(&current, requested)? {
            if existing.file_name() != Some(component) {
                return Err(format!(
                    "project scene directory differs only by case from existing path: {}",
                    existing.display()
                ));
            }
            let metadata = std::fs::symlink_metadata(&existing)
                .map_err(|error| format!("could not inspect {}: {error}", existing.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "project scene directory is not a real directory: {}",
                    existing.display()
                ));
            }
            current = existing;
            continue;
        }
        if !create_missing {
            return Err(format!(
                "project scene directory does not exist: {}",
                current.join(component).display()
            ));
        }
        let created = current.join(component);
        match std::fs::create_dir(&created) {
            Ok(()) => current = created,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = std::fs::symlink_metadata(&created).map_err(|inspect_error| {
                    format!("could not inspect {}: {inspect_error}", created.display())
                })?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(format!(
                        "project scene directory is not a real directory: {}",
                        created.display()
                    ));
                }
                current = created;
            }
            Err(error) => {
                return Err(format!(
                    "could not create project scene directory {}: {error}",
                    created.display()
                ));
            }
        }
    }
    Ok(())
}

pub(crate) fn portable_scene_path_key(path: &Path) -> String {
    path.components()
        .filter_map(|component| match component {
            Component::Normal(value) => value.to_str(),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
        .to_ascii_lowercase()
}

pub(crate) fn portable_path_string(path: &Path) -> Result<String, String> {
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| "project scene path contains non-UTF-8 text".to_string())?
                    .to_string(),
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "project scene path is not project-relative: {}",
                    path.display()
                ));
            }
        }
    }
    if parts.is_empty() {
        Err("project scene path may not be empty".into())
    } else {
        Ok(parts.join("/"))
    }
}

pub(crate) fn portable_project_relative_path(
    project_root: &Path,
    path: &Path,
) -> Result<String, String> {
    let relative = path.strip_prefix(project_root).map_err(|_| {
        format!(
            "scene path is outside the project root and cannot be recorded: {}",
            path.display()
        )
    })?;
    portable_path_string(relative)
}

pub(crate) fn unix_nanos() -> Result<u128, String> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .map_err(|error| format!("system clock is before the Unix epoch: {error}"))
}

pub(crate) fn allocate_scene_trash_directory(
    project: &GameProject,
    scene_id: &str,
    deleted_unix_nanos: u128,
) -> Result<PathBuf, String> {
    let trash_root = project.root.join(".engine/trash/scenes");
    ensure_scene_directory_chain(&project.root, &trash_root, true)?;
    for attempt in 0..100usize {
        let candidate = trash_root.join(format!(
            "{deleted_unix_nanos}-{scene_id}-{}-{attempt}",
            std::process::id()
        ));
        if find_case_insensitive_entry(
            &trash_root,
            candidate
                .file_name()
                .and_then(|name| name.to_str())
                .expect("generated scene trash name is portable UTF-8"),
        )?
        .is_none()
        {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not allocate a unique scene trash directory below {}",
        trash_root.display()
    ))
}
