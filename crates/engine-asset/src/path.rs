use std::fmt;
use std::path::{Path, PathBuf};

use engine_serialize::AssetId;

/// Why an [`AssetId`] cannot be mapped to a portable package path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetPathError {
    pub field: &'static str,
    pub value: String,
    pub reason: String,
}

impl fmt::Display for AssetPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "invalid {} `{}`: {}",
            self.field, self.value, self.reason
        )
    }
}

impl std::error::Error for AssetPathError {}

/// Validate both the identifier and optional logical path using portable
/// package path rules. Windows path syntax is rejected on every host.
pub fn validate_asset_id(id: &AssetId) -> Result<(), AssetPathError> {
    validate_portable_relative(&id.id, "AssetId.id", true)?;
    if let Some(logical_path) = &id.logical_path {
        validate_portable_relative(logical_path, "AssetId.logical_path", false)?;
    }
    Ok(())
}

/// Return the canonical package-relative path, always using `/` separators.
///
/// Assets in a hot-update package live below `assets/`, for example
/// `assets/meshes/cube.asset`.
pub fn asset_relative_path(id: &AssetId) -> Result<String, AssetPathError> {
    validate_asset_id(id)?;
    if let Some(logical_path) = &id.logical_path {
        return Ok(format!("assets/{logical_path}"));
    }

    let id_str = id.id.as_str();
    if let Some(hyphen_pos) = id_str.find('-') {
        let category = &id_str[..hyphen_pos];
        let name = &id_str[hyphen_pos + 1..];
        if category.is_empty() || name.is_empty() {
            return Err(invalid(
                "AssetId.id",
                id_str,
                "category and name around `-` must both be non-empty",
            ));
        }
        let directory = match category {
            "mesh" => "meshes",
            "material" => "materials",
            "texture" => "textures",
            "shader" => "shaders",
            "scene" => "scenes",
            "prefab" => "prefabs",
            "animation" => "animations",
            "audio" => "audio",
            "font" => "fonts",
            "logic" => "logic",
            "pipeline" => "pipelines",
            "navmesh" => "navmeshes",
            "script" => "scripts",
            "skeleton" => "skeletons",
            other => other,
        };
        Ok(format!("assets/{directory}/{name}.asset"))
    } else {
        Ok(format!("assets/{id_str}.asset"))
    }
}

/// Resolve an [`AssetId`] relative to the process working directory.
///
/// Invalid or non-portable identifiers return `None` for compatibility with
/// the existing registry API.
pub fn asset_path(id: &AssetId) -> Option<PathBuf> {
    asset_relative_path(id).ok().map(PathBuf::from)
}

/// Resolve an [`AssetId`] below an explicit content root.
pub fn asset_path_from_root(root: &Path, id: &AssetId) -> Result<PathBuf, AssetPathError> {
    asset_relative_path(id).map(|relative| root.join(relative))
}

fn validate_portable_relative(
    value: &str,
    field: &'static str,
    single_component: bool,
) -> Result<(), AssetPathError> {
    if value.is_empty() {
        return Err(invalid(field, value, "value is empty"));
    }
    if value.chars().any(char::is_control) {
        return Err(invalid(field, value, "control characters are forbidden"));
    }
    if value.contains('\\') {
        return Err(invalid(
            field,
            value,
            "backslashes and UNC paths are forbidden",
        ));
    }
    if value.starts_with('/') || Path::new(value).is_absolute() {
        return Err(invalid(field, value, "absolute paths are forbidden"));
    }
    let bytes = value.as_bytes();
    if bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return Err(invalid(
            field,
            value,
            "Windows drive prefixes are forbidden",
        ));
    }
    if value.contains(':') {
        return Err(invalid(
            field,
            value,
            "colons and alternate data streams are forbidden",
        ));
    }
    if single_component && value.contains('/') {
        return Err(invalid(
            field,
            value,
            "path separators are forbidden in asset identifiers",
        ));
    }

    for component in value.split('/') {
        if component.is_empty() {
            return Err(invalid(field, value, "empty path components are forbidden"));
        }
        if matches!(component, "." | "..") {
            return Err(invalid(field, value, "`.` and `..` are forbidden"));
        }
        if component.ends_with(['.', ' ']) {
            return Err(invalid(
                field,
                value,
                "components ending in dot or space are forbidden",
            ));
        }
        if is_windows_device_name(component) {
            return Err(invalid(field, value, "Windows device names are forbidden"));
        }
    }
    Ok(())
}

fn invalid(field: &'static str, value: &str, reason: &str) -> AssetPathError {
    AssetPathError {
        field,
        value: value.to_string(),
        reason: reason.to_string(),
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
