use std::path::{Path, PathBuf};

use engine_serialize::HotUpdateManifest;

use crate::error::UpdateError;

const WINDOWS_REPARSE_POINT_ATTRIBUTE: u32 = 0x0400;

fn unsafe_path(field: &str, path: &str, reason: impl Into<String>) -> UpdateError {
    UpdateError::UnsafePath {
        field: field.to_string(),
        path: path.to_string(),
        reason: reason.into(),
    }
}

/// Validate a portable, non-empty relative manifest path.
///
/// Manifest paths always use `/` as their separator. Rejecting Windows path
/// syntax even on Unix keeps a package from becoming unsafe when it is moved
/// between build and target hosts.
pub(crate) fn validate_relative_path(value: &str, field: &str) -> Result<(), UpdateError> {
    if value.is_empty() {
        return Err(unsafe_path(field, value, "path is empty"));
    }
    if value.chars().any(char::is_control) {
        return Err(unsafe_path(
            field,
            value,
            "control characters are forbidden",
        ));
    }
    if value.contains('\\') {
        return Err(unsafe_path(
            field,
            value,
            "backslashes and UNC paths are forbidden",
        ));
    }
    if value.starts_with('/') || Path::new(value).is_absolute() {
        return Err(unsafe_path(field, value, "absolute paths are forbidden"));
    }
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(unsafe_path(
            field,
            value,
            "Windows drive prefixes are forbidden",
        ));
    }
    if value.contains(':') {
        return Err(unsafe_path(
            field,
            value,
            "colons and Windows alternate data streams are forbidden",
        ));
    }

    for component in value.split('/') {
        if component.is_empty() {
            return Err(unsafe_path(
                field,
                value,
                "empty path components are forbidden",
            ));
        }
        if component == "." || component == ".." {
            return Err(unsafe_path(
                field,
                value,
                "`.` and `..` components are forbidden",
            ));
        }
        if component.ends_with(['.', ' ']) {
            return Err(unsafe_path(
                field,
                value,
                "components ending in a dot or space are forbidden",
            ));
        }
        if is_windows_device_name(component) {
            return Err(unsafe_path(
                field,
                value,
                "Windows device names are forbidden",
            ));
        }
    }

    Ok(())
}

/// Validate a value that is used as exactly one filesystem component.
pub(crate) fn validate_component(value: &str, field: &str) -> Result<(), UpdateError> {
    validate_relative_path(value, field)?;
    if value.contains('/') {
        return Err(unsafe_path(
            field,
            value,
            "path separators are forbidden in identifiers",
        ));
    }
    Ok(())
}

pub(crate) fn validate_package_id(package_id: &str) -> Result<(), UpdateError> {
    validate_component(package_id, "package_id")
}

/// Validate every manifest-controlled value that is later used in a path.
pub(crate) fn validate_manifest_paths(
    manifest: &HotUpdateManifest,
) -> Result<(), Vec<UpdateError>> {
    let mut errors = Vec::new();

    for payload_hash in &manifest.payload_hashes {
        if let Err(error) = validate_relative_path(&payload_hash.path, "payload_hashes[].path") {
            errors.push(error);
        }
    }

    for payload in &manifest.platform_payloads {
        for asset_id in &payload.asset_ids {
            if let Err(error) =
                validate_component(&asset_id.id, "platform_payloads[].asset_ids[].id")
            {
                errors.push(error);
            }
            if let Some(logical_path) = &asset_id.logical_path {
                if let Err(error) = validate_relative_path(
                    logical_path,
                    "platform_payloads[].asset_ids[].logical_path",
                ) {
                    errors.push(error);
                }
            }
        }
        for logic_id in &payload.logic_asset_ids {
            if let Err(error) =
                validate_component(logic_id, "platform_payloads[].logic_asset_ids[]")
            {
                errors.push(error);
            }
        }
        if let Some(assembly) = &payload.optional_assembly {
            if let Err(error) =
                validate_relative_path(&assembly.path, "platform_payloads[].optional_assembly.path")
            {
                errors.push(error);
            }
        }
    }

    if let Some(path) = &manifest.rollback.fallback_manifest_path {
        if let Err(error) = validate_relative_path(path, "rollback.fallback_manifest_path") {
            errors.push(error);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub(crate) fn validate_manifest_paths_once(
    manifest: &HotUpdateManifest,
) -> Result<(), UpdateError> {
    match validate_manifest_paths(manifest) {
        Ok(()) => Ok(()),
        Err(errors) => Err(errors
            .into_iter()
            .next()
            .unwrap_or_else(|| unsafe_path("manifest", "", "manifest path validation failed"))),
    }
}

/// Join an already validated relative path while rejecting links/reparse
/// points in every existing prefix.
pub(crate) fn safe_join(base: &Path, relative: &str, field: &str) -> Result<PathBuf, UpdateError> {
    validate_relative_path(relative, field)?;
    ensure_no_links_in_path(base, field)?;
    let joined = base.join(relative);
    ensure_no_links_in_path(&joined, field)?;
    Ok(joined)
}

pub(crate) fn safe_package_path(
    base: &Path,
    area: &str,
    package_id: &str,
) -> Result<PathBuf, UpdateError> {
    validate_component(area, "cache area")?;
    validate_package_id(package_id)?;
    safe_join(base, &format!("{area}/{package_id}"), "cache package path")
}

/// Reject a symlink or Windows reparse point anywhere in an existing path.
pub(crate) fn ensure_no_links_in_path(path: &Path, field: &str) -> Result<(), UpdateError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match std::fs::symlink_metadata(&current) {
            Ok(metadata) if is_link_or_reparse(&metadata) => {
                return Err(unsafe_path(
                    field,
                    &path.display().to_string(),
                    format!("link or reparse point in path: {}", current.display()),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(UpdateError::Io(error)),
        }
    }
    Ok(())
}

/// Recursively validate a directory tree without following links.
pub(crate) fn ensure_tree_has_no_links(root: &Path, field: &str) -> Result<(), UpdateError> {
    ensure_no_links_in_path(root, field)?;
    let metadata = std::fs::symlink_metadata(root)?;
    if is_link_or_reparse(&metadata) {
        return Err(unsafe_path(
            field,
            &root.display().to_string(),
            "directory is a link or reparse point",
        ));
    }
    if !metadata.is_dir() {
        return Err(unsafe_path(
            field,
            &root.display().to_string(),
            "expected a directory",
        ));
    }

    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            unsafe_path(
                field,
                &path.display().to_string(),
                "non-UTF-8 file names are forbidden in package trees",
            )
        })?;
        validate_component(name, field)?;
        let metadata = std::fs::symlink_metadata(&path)?;
        if is_link_or_reparse(&metadata) {
            return Err(unsafe_path(
                field,
                &path.display().to_string(),
                "directory tree contains a link or reparse point",
            ));
        }
        if metadata.is_dir() {
            ensure_tree_has_no_links(&path, field)?;
        } else if !metadata.is_file() {
            return Err(unsafe_path(
                field,
                &path.display().to_string(),
                "directory tree contains a non-regular file",
            ));
        }
    }
    Ok(())
}

pub(crate) fn remove_dir_all_safe(path: &Path, field: &str) -> Result<(), UpdateError> {
    ensure_no_links_in_path(path, field)?;
    if path.exists() {
        ensure_tree_has_no_links(path, field)?;
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}

pub(crate) fn is_link_or_reparse(metadata: &std::fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & WINDOWS_REPARSE_POINT_ATTRIBUTE != 0
    }

    #[cfg(not(windows))]
    {
        let _ = WINDOWS_REPARSE_POINT_ATTRIBUTE;
        false
    }
}

fn is_windows_device_name(component: &str) -> bool {
    let stem = component.split('.').next().unwrap_or(component);
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (upper.len() == 4
            && (upper.starts_with("COM") || upper.starts_with("LPT"))
            && matches!(upper.as_bytes()[3], b'1'..=b'9'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn portable_relative_path_rejects_escape_and_windows_syntax() {
        for path in [
            "",
            "../x",
            "/abs",
            "C:\\x",
            "C:/x",
            "\\\\server\\share",
            "//server/share",
            "a\\..\\b",
            "a\\\\b",
            "a/../b",
            "a/./b",
            "a//b",
            "nul",
            "a\0b",
        ] {
            assert!(
                validate_relative_path(path, "test").is_err(),
                "unsafe path accepted: {path:?}"
            );
        }
    }

    #[test]
    fn portable_relative_path_accepts_nested_paths() {
        for path in ["file.bin", "a/deep/nested/file.cooked", "unicode/资源.bin"] {
            assert!(
                validate_relative_path(path, "test").is_ok(),
                "valid path rejected: {path:?}"
            );
        }
    }

    #[test]
    fn package_id_must_be_one_safe_component() {
        for id in ["../victim", "a/b", "a\\b", ".", "..", "C:evil"] {
            assert!(validate_package_id(id).is_err(), "unsafe id accepted: {id}");
        }
        assert!(validate_package_id("test-pkg_01.abc").is_ok());
    }

    #[cfg(windows)]
    #[test]
    fn safe_join_rejects_windows_reparse_points() {
        use std::os::windows::fs::symlink_file;

        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("target.bin");
        let link = temp.path().join("link.bin");
        std::fs::write(&target, b"target").unwrap();

        // Creating symlinks may require Developer Mode or an elevated test
        // process. The assertion runs whenever the host permits creation.
        if symlink_file(&target, &link).is_ok() {
            assert!(safe_join(temp.path(), "link.bin", "test").is_err());
        }
    }
}
