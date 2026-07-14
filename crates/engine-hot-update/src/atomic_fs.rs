//! Crash-resistant writes for the cache's small metadata files.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::UpdateError;

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(1);

/// Write `contents` through a same-directory temporary file, flush it, and
/// atomically replace `path`.
pub(crate) fn atomic_write(path: &Path, contents: &[u8]) -> Result<(), UpdateError> {
    let parent = path.parent().ok_or_else(|| {
        UpdateError::CacheCorrupt(format!("metadata path has no parent: {}", path.display()))
    })?;
    std::fs::create_dir_all(parent)?;

    let (temp_path, mut temp_file) = create_temp_file(path)?;
    let result = (|| {
        temp_file.write_all(contents)?;
        temp_file.sync_all()?;
        drop(temp_file);
        replace_file(&temp_path, path)?;
        sync_directory(parent)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&temp_path);
    }
    result
}

fn create_temp_file(path: &Path) -> Result<(PathBuf, File), UpdateError> {
    let parent = path.parent().ok_or_else(|| {
        UpdateError::CacheCorrupt(format!("metadata path has no parent: {}", path.display()))
    })?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            UpdateError::CacheCorrupt(format!("metadata path is not UTF-8: {}", path.display()))
        })?;

    for _ in 0..32 {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), id));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(UpdateError::Io(error)),
        }
    }

    Err(UpdateError::CacheCorrupt(format!(
        "could not allocate temporary metadata file for {}",
        path.display()
    )))
}

#[cfg(unix)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    std::fs::rename(source, destination)?;
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source_wide: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: both buffers are NUL-terminated and remain alive for the call.
    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| UpdateError::Io(std::io::Error::other(error.to_string())))
}

#[cfg(not(any(unix, windows)))]
fn replace_file(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    if destination.exists() {
        return Err(UpdateError::CacheCorrupt(
            "atomic replace is unsupported on this target".into(),
        ));
    }
    std::fs::rename(source, destination)?;
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), UpdateError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), UpdateError> {
    Ok(())
}

/// Flush a prepared payload tree before it can become reachable through the
/// active pointer. This closes the ordering gap where a directory rename is
/// durable but recently copied file contents are not.
pub(crate) fn sync_tree(path: &Path) -> Result<(), UpdateError> {
    let metadata = std::fs::symlink_metadata(path)?;
    if metadata.is_file() {
        sync_regular_file(path)?;
        return Ok(());
    }
    if !metadata.is_dir() {
        return Err(UpdateError::CacheCorrupt(format!(
            "cannot sync non-file payload entry: {}",
            path.display()
        )));
    }
    for entry in std::fs::read_dir(path)? {
        sync_tree(&entry?.path())?;
    }
    sync_directory(path)
}

#[cfg(windows)]
fn sync_regular_file(path: &Path) -> Result<(), UpdateError> {
    // FlushFileBuffers requires a handle opened with write access on Windows;
    // File::open creates a read-only handle and fails with ERROR_ACCESS_DENIED.
    OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()?;
    Ok(())
}

#[cfg(not(windows))]
fn sync_regular_file(path: &Path) -> Result<(), UpdateError> {
    File::open(path)?.sync_all()?;
    Ok(())
}

/// Move a fully prepared directory and durably order the move before a later
/// active-pointer commit.
pub(crate) fn durable_rename_directory(
    source: &Path,
    destination: &Path,
) -> Result<(), UpdateError> {
    sync_tree(source)?;
    rename_directory(source, destination)?;
    if let Some(parent) = source.parent() {
        sync_directory(parent)?;
    }
    if destination.parent() != source.parent() {
        if let Some(parent) = destination.parent() {
            sync_directory(parent)?;
        }
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn rename_directory(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_WRITE_THROUGH};

    let source_wide: Vec<u16> = source
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    // SAFETY: both buffers are NUL-terminated and live through the call.
    unsafe {
        MoveFileExW(
            PCWSTR(source_wide.as_ptr()),
            PCWSTR(destination_wide.as_ptr()),
            MOVEFILE_WRITE_THROUGH,
        )
    }
    .map_err(|error| UpdateError::Io(std::io::Error::other(error.to_string())))
}

#[cfg(not(windows))]
fn rename_directory(source: &Path, destination: &Path) -> Result<(), UpdateError> {
    std::fs::rename(source, destination)?;
    Ok(())
}

pub(crate) fn remove_file_if_exists(path: &Path) -> Result<(), UpdateError> {
    match std::fs::remove_file(path) {
        Ok(()) => {
            if let Some(parent) = path.parent() {
                sync_directory(parent)?;
            }
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(UpdateError::Io(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_creates_and_replaces() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("state.json");
        atomic_write(&path, b"first").unwrap();
        atomic_write(&path, b"second").unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"second");
    }

    #[test]
    fn durable_directory_rename_preserves_prepared_payload() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("staged").join("package");
        let destination = temp.path().join("active").join("package");
        std::fs::create_dir_all(source.join("assets")).unwrap();
        std::fs::create_dir_all(destination.parent().unwrap()).unwrap();
        std::fs::write(source.join("assets/data.bin"), b"verified payload").unwrap();

        durable_rename_directory(&source, &destination).unwrap();

        assert!(!source.exists());
        assert_eq!(
            std::fs::read(destination.join("assets/data.bin")).unwrap(),
            b"verified payload"
        );
    }
}
