#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WriteMode {
    Create,
    Replace,
    CurrentState,
}

pub(super) struct CommitWrite {
    path: PathBuf,
    bytes: Vec<u8>,
    mode: WriteMode,
}

impl CommitWrite {
    pub(super) fn create(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: WriteMode::Create,
        }
    }

    pub(super) fn replace(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: WriteMode::Replace,
        }
    }

    pub(super) fn for_current_state(path: PathBuf, bytes: Vec<u8>) -> Self {
        Self {
            path,
            bytes,
            mode: WriteMode::CurrentState,
        }
    }
}

struct FileSnapshot {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}

pub(super) fn commit_transaction(
    project_root: &Path,
    writes: Vec<CommitWrite>,
    deletes: Vec<PathBuf>,
    fail_after_mutation: Option<usize>,
) -> Result<(), String> {
    let mut unique_paths = BTreeSet::new();
    let mut snapshots = Vec::new();
    for path in writes
        .iter()
        .map(|write| write.path.as_path())
        .chain(deletes.iter().map(PathBuf::as_path))
    {
        if !path.starts_with(project_root) {
            return Err(format!(
                "asset transaction target escapes project root: {}",
                path.display()
            ));
        }
        ensure_no_symlink_ancestors(project_root, path)?;
        if unique_paths.insert(path.to_path_buf()) {
            let bytes = if path.is_file() {
                Some(std::fs::read(path).map_err(io_read(path))?)
            } else if path.exists() {
                return Err(format!(
                    "asset transaction target is not a regular file: {}",
                    path.display()
                ));
            } else {
                None
            };
            snapshots.push(FileSnapshot {
                path: path.to_path_buf(),
                bytes,
            });
        }
    }
    for write in &writes {
        let existed = snapshots
            .iter()
            .find(|snapshot| snapshot.path == write.path)
            .and_then(|snapshot| snapshot.bytes.as_ref())
            .is_some();
        match write.mode {
            WriteMode::Create if existed => {
                return Err(format!(
                    "asset transaction will not overwrite existing file: {}",
                    write.path.display()
                ));
            }
            WriteMode::Replace if !existed => {
                return Err(format!(
                    "asset transaction expected an existing file: {}",
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
                "asset transaction cannot delete missing file: {}",
                delete.display()
            ));
        }
    }

    let mut mutations = 0usize;
    let result = (|| {
        for write in &writes {
            if write.mode == WriteMode::Create {
                write_file_create_new(&write.path, &write.bytes)?;
            } else {
                crate::project_cli::atomic_write_bytes(&write.path, &write.bytes)?;
            }
            mutations += 1;
            maybe_inject_commit_failure(fail_after_mutation, mutations)?;
        }
        for delete in &deletes {
            std::fs::remove_file(delete)
                .map_err(|error| format!("could not remove {}: {error}", delete.display()))?;
            mutations += 1;
            maybe_inject_commit_failure(fail_after_mutation, mutations)?;
        }
        Ok(())
    })();
    if let Err(failure) = result {
        let rollback_errors = restore_snapshots(&snapshots);
        remove_empty_created_parents(project_root, &snapshots);
        return if rollback_errors.is_empty() {
            Err(failure)
        } else {
            Err(format!(
                "{failure}\nasset transaction rollback also failed:\n{}",
                rollback_errors.join("\n")
            ))
        };
    }
    Ok(())
}

fn restore_snapshots(snapshots: &[FileSnapshot]) -> Vec<String> {
    let mut errors = Vec::new();
    for snapshot in snapshots.iter().rev() {
        let restored = match &snapshot.bytes {
            Some(bytes) => crate::project_cli::atomic_write_bytes(&snapshot.path, bytes),
            None if snapshot.path.is_file() => std::fs::remove_file(&snapshot.path)
                .map_err(|error| format!("could not remove {}: {error}", snapshot.path.display())),
            None if snapshot.path.exists() => Err(format!(
                "rollback target became a non-file: {}",
                snapshot.path.display()
            )),
            None => Ok(()),
        };
        if let Err(error) = restored {
            errors.push(error);
        }
    }
    errors
}

fn maybe_inject_commit_failure(
    fail_after_mutation: Option<usize>,
    mutations: usize,
) -> Result<(), String> {
    if fail_after_mutation == Some(mutations) {
        Err(format!(
            "simulated asset transaction failure after mutation {mutations}"
        ))
    } else {
        Ok(())
    }
}

fn remove_empty_created_parents(project_root: &Path, snapshots: &[FileSnapshot]) {
    let mut directories = snapshots
        .iter()
        .filter(|snapshot| snapshot.bytes.is_none())
        .filter_map(|snapshot| snapshot.path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    directories.dedup();
    for mut directory in directories {
        while directory.starts_with(project_root) && directory != project_root {
            if std::fs::remove_dir(&directory).is_err() {
                break;
            }
            let Some(parent) = directory.parent() else {
                break;
            };
            directory = parent.to_path_buf();
        }
    }
}
use super::*;
