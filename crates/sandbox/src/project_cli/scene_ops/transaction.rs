use super::*;
use std::fs::OpenOptions;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SceneWriteMode {
    Create,
    Replace,
}

#[derive(Clone, Debug)]
pub(crate) struct SceneTransactionWrite {
    path: PathBuf,
    bytes: Vec<u8>,
    mode: SceneWriteMode,
}

impl SceneTransactionWrite {
    pub(crate) fn create(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: SceneWriteMode::Create,
        }
    }

    pub(crate) fn replace(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: SceneWriteMode::Replace,
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct SceneFileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

pub(crate) fn commit_scene_transaction(
    project_root: &Path,
    writes: Vec<SceneTransactionWrite>,
    deletes: Vec<PathBuf>,
    fail_after_mutation: Option<usize>,
) -> Result<(), String> {
    let mut snapshots = Vec::<SceneFileSnapshot>::new();
    let mut touched_paths = BTreeSet::new();
    for path in writes.iter().map(|write| &write.path).chain(deletes.iter()) {
        let relative = path.strip_prefix(project_root).map_err(|_| {
            format!(
                "scene transaction path escapes project root: {}",
                path.display()
            )
        })?;
        if relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        }) {
            return Err(format!(
                "scene transaction path is not normalized: {}",
                path.display()
            ));
        }
        let portable_key = portable_scene_path_key(relative);
        if !touched_paths.insert(portable_key) {
            return Err(format!(
                "scene transaction touches the same portable path more than once: {}",
                path.display()
            ));
        }
        ensure_no_scene_symlink_ancestors(project_root, path)?;
        let bytes = if path.exists() {
            let metadata = std::fs::symlink_metadata(path)
                .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                return Err(format!(
                    "scene transaction target is not a regular file: {}",
                    path.display()
                ));
            }
            Some(
                std::fs::read(path)
                    .map_err(|error| format!("could not snapshot {}: {error}", path.display()))?,
            )
        } else {
            None
        };
        snapshots.push(SceneFileSnapshot {
            path: path.clone(),
            bytes,
        });
    }

    for write in &writes {
        let existed = snapshots
            .iter()
            .find(|snapshot| snapshot.path == write.path)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_some();
        match write.mode {
            SceneWriteMode::Create if existed => {
                return Err(format!(
                    "scene transaction will not overwrite existing file: {}",
                    write.path.display()
                ));
            }
            SceneWriteMode::Replace if !existed => {
                return Err(format!(
                    "scene transaction expected an existing file: {}",
                    write.path.display()
                ));
            }
            _ => {}
        }
    }
    for delete in &deletes {
        if snapshots
            .iter()
            .find(|snapshot| snapshot.path == *delete)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_none()
        {
            return Err(format!(
                "scene transaction cannot move missing file: {}",
                delete.display()
            ));
        }
    }

    let mut mutations = 0usize;
    let result = (|| {
        for write in &writes {
            match write.mode {
                SceneWriteMode::Create => write_bytes_create_new(&write.path, &write.bytes)?,
                SceneWriteMode::Replace => atomic_write_bytes(&write.path, &write.bytes)?,
            }
            mutations += 1;
            maybe_inject_scene_commit_failure(fail_after_mutation, mutations)?;
        }
        for delete in &deletes {
            std::fs::remove_file(delete)
                .map_err(|error| format!("could not move {}: {error}", delete.display()))?;
            mutations += 1;
            maybe_inject_scene_commit_failure(fail_after_mutation, mutations)?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        let rollback_errors = restore_scene_snapshots(&snapshots);
        return if rollback_errors.is_empty() {
            Err(error)
        } else {
            Err(format!(
                "{error}\nscene transaction rollback also failed:\n{}",
                rollback_errors.join("\n")
            ))
        };
    }
    Ok(())
}

pub(crate) fn maybe_inject_scene_commit_failure(
    fail_after_mutation: Option<usize>,
    mutations: usize,
) -> Result<(), String> {
    if fail_after_mutation == Some(mutations) {
        Err(format!(
            "injected scene transaction failure after mutation {mutations}"
        ))
    } else {
        Ok(())
    }
}

pub(crate) fn restore_scene_snapshots(snapshots: &[SceneFileSnapshot]) -> Vec<String> {
    let mut errors = Vec::new();
    for snapshot in snapshots.iter().rev() {
        let result = match &snapshot.bytes {
            Some(bytes) => atomic_write_bytes(&snapshot.path, bytes),
            None if snapshot.path.exists() => std::fs::remove_file(&snapshot.path)
                .map_err(|error| format!("could not remove {}: {error}", snapshot.path.display())),
            None => Ok(()),
        };
        if let Err(error) = result {
            errors.push(error);
        }
    }
    errors
}

pub(crate) fn write_bytes_create_new(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    if !parent.is_dir() {
        return Err(format!(
            "scene transaction parent directory does not exist: {}",
            parent.display()
        ));
    }
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

pub(crate) fn serialize_scene(scene: &Scene) -> Result<Vec<u8>, String> {
    ron::ser::to_string_pretty(scene, ron::ser::PrettyConfig::default())
        .map(String::into_bytes)
        .map_err(|error| format!("could not serialize scene '{}': {error}", scene.scene_id))
}

pub(crate) fn serialize_project_manifest(manifest: &ProjectManifest) -> Result<Vec<u8>, String> {
    manifest
        .validate()
        .map_err(|error| format!("invalid project manifest update: {error}"))?;
    let mut serialized = serde_json::to_vec_pretty(manifest)
        .map_err(|error| format!("could not serialize project manifest: {error}"))?;
    serialized.push(b'\n');
    Ok(serialized)
}

pub(crate) fn atomic_write_project_manifest(
    manifest: &ProjectManifest,
    path: &Path,
) -> Result<(), String> {
    atomic_write_bytes(path, &serialize_project_manifest(manifest)?)
}

pub(crate) fn atomic_write_bytes(path: &Path, contents: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| format!("path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|error| format!("could not create {}: {error}", parent.display()))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|error| {
        format!(
            "could not create a temporary file beside {}: {error}",
            path.display()
        )
    })?;
    temporary
        .write_all(contents)
        .map_err(|error| format!("could not write temporary {}: {error}", path.display()))?;
    temporary
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not flush temporary {}: {error}", path.display()))?;
    let persisted = temporary.persist(path).map_err(|error| {
        format!(
            "could not atomically replace {}: {}",
            path.display(),
            error.error
        )
    })?;
    persisted
        .sync_all()
        .map_err(|error| format!("could not flush {}: {error}", path.display()))?;
    Ok(())
}
