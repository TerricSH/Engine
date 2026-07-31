use super::*;
use std::fs::OpenOptions;

pub(crate) fn is_supported_texture_extension(extension: &str) -> bool {
    matches!(
        extension,
        "png"
            | "jpg"
            | "jpeg"
            | "bmp"
            | "gif"
            | "ico"
            | "tif"
            | "tiff"
            | "webp"
            | "pnm"
            | "pbm"
            | "pgm"
            | "ppm"
            | "pam"
            | "tga"
            | "hdr"
            | "qoi"
            | "exr"
    )
}

pub(crate) fn import_asset_type_label(asset_type: &AssetType) -> &'static str {
    match asset_type {
        AssetType::Mesh => "mesh",
        AssetType::Texture => "texture",
        AssetType::Material => "material",
        AssetType::EnvironmentMap => "environment",
        AssetType::Audio => "audio",
        AssetType::Animation => "animation",
        AssetType::Skeleton => "skeleton",
        AssetType::NavMesh => "navmesh",
        AssetType::Prefab => "prefab",
        _ => "unsupported",
    }
}

pub(crate) fn validate_import_asset_id(asset_id: &str) -> Result<(), String> {
    if asset_id.is_empty() || asset_id.len() > 128 {
        return Err("asset id must contain between 1 and 128 ASCII characters".into());
    }
    if !asset_id
        .chars()
        .next()
        .is_some_and(|character| character.is_ascii_alphanumeric())
    {
        return Err("asset id must start with an ASCII letter or digit".into());
    }
    if !asset_id
        .chars()
        .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    {
        return Err(
            "asset id may contain only ASCII letters, digits, hyphens, underscores, and dots"
                .into(),
        );
    }
    let portable_stem = asset_id
        .split('.')
        .next()
        .unwrap_or(asset_id)
        .to_ascii_uppercase();
    if matches!(portable_stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (portable_stem.len() == 4
            && (portable_stem.starts_with("COM") || portable_stem.starts_with("LPT"))
            && portable_stem.as_bytes()[3].is_ascii_digit()
            && portable_stem.as_bytes()[3] != b'0')
    {
        return Err(format!(
            "asset id '{asset_id}' is not portable because it uses a reserved file name"
        ));
    }
    Ok(())
}

pub(crate) fn normalize_existing_import_folder(
    source_root: &Path,
    folder: &Path,
) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    let mut current = source_root.to_path_buf();
    for component in folder.components() {
        let Component::Normal(component) = component else {
            return Err(format!(
                "asset import folder must be a portable project-relative path: {}",
                folder.display()
            ));
        };
        let requested = component
            .to_str()
            .ok_or_else(|| "asset import folder contains non-UTF-8 text".to_string())?;
        if requested.is_empty()
            || requested.ends_with([' ', '.'])
            || requested.chars().any(|character| {
                character.is_control()
                    || matches!(
                        character,
                        '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*'
                    )
            })
        {
            return Err(format!(
                "asset import folder contains a non-portable component '{requested}'"
            ));
        }
        let stem = requested
            .split('.')
            .next()
            .unwrap_or(requested)
            .to_ascii_uppercase();
        if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (stem.len() == 4
                && (stem.starts_with("COM") || stem.starts_with("LPT"))
                && stem.as_bytes()[3].is_ascii_digit()
                && stem.as_bytes()[3] != b'0')
        {
            return Err(format!(
                "asset import folder uses reserved portable name '{requested}'"
            ));
        }
        let existing = find_case_insensitive_entry(&current, requested)?.ok_or_else(|| {
            format!(
                "asset import folder does not exist: {}",
                current.join(component).display()
            )
        })?;
        if existing.file_name() != Some(component) {
            return Err(format!(
                "asset import folder differs only by case from existing folder: {}",
                existing.display()
            ));
        }
        let metadata = std::fs::symlink_metadata(&existing)
            .map_err(|error| format!("could not inspect {}: {error}", existing.display()))?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(format!(
                "asset import folder is not a real directory: {}",
                existing.display()
            ));
        }
        normalized.push(component);
        current = existing;
    }
    Ok(normalized)
}

pub(crate) fn find_case_insensitive_entry(
    directory: &Path,
    name: &str,
) -> Result<Option<PathBuf>, String> {
    if !directory.exists() {
        return Ok(None);
    }
    if !directory.is_dir() {
        return Err(format!("expected a directory: {}", directory.display()));
    }
    for entry in std::fs::read_dir(directory)
        .map_err(|error| format!("could not enumerate {}: {error}", directory.display()))?
    {
        let entry = entry
            .map_err(|error| format!("could not enumerate {}: {error}", directory.display()))?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        {
            return Ok(Some(entry.path()));
        }
    }
    Ok(None)
}

pub(crate) fn copy_file_create_new(source: &Path, target: &Path) -> Result<(), String> {
    let mut input = std::fs::File::open(source)
        .map_err(|error| format!("could not open {}: {error}", source.display()))?;
    let mut output = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| {
            format!(
                "could not create {} without overwriting an existing file: {error}",
                target.display()
            )
        })?;
    if let Err(error) = std::io::copy(&mut input, &mut output) {
        drop(output);
        let _ = std::fs::remove_file(target);
        return Err(format!(
            "could not copy {} to {}: {error}",
            source.display(),
            target.display()
        ));
    }
    Ok(())
}

pub(crate) fn import_staging_directory(project: &GameProject) -> Result<PathBuf, String> {
    let parent = project
        .cooked_assets
        .parent()
        .unwrap_or(project.root.as_path());
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    for attempt in 0..100_u32 {
        let candidate = parent.join(format!(
            ".asset-import-cook-{}-{unique}-{attempt}",
            std::process::id()
        ));
        if !candidate.exists() && candidate.starts_with(&project.root) {
            return Ok(candidate);
        }
    }
    Err("could not allocate a unique asset import staging directory".into())
}

pub(crate) fn remove_import_staging(project: &GameProject, staging: &Path) {
    if staging.starts_with(&project.root) && staging.exists() {
        let _ = std::fs::remove_dir_all(staging);
    }
}

pub(crate) fn cook_report_failure(report: &engine_asset::cook::CookReport) -> String {
    let details = report
        .diagnostics
        .iter()
        .filter(|diagnostic| {
            matches!(
                diagnostic.severity,
                DiagnosticSeverity::Error | DiagnosticSeverity::Fatal
            )
        })
        .take(8)
        .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
        .collect::<Vec<_>>()
        .join("\n");
    if details.is_empty() {
        "project asset cooking failed during import".into()
    } else {
        format!("project asset cooking failed during import:\n{details}")
    }
}

pub(crate) fn rollback_import_failure(
    failure: String,
    manifest_path: &Path,
    original_manifest: Option<&[u8]>,
    copied_sources: &[PathBuf],
    copied_cooked: &[PathBuf],
) -> String {
    let mut rollback_errors = Vec::new();
    let manifest_restore = match original_manifest {
        Some(content) => std::fs::write(manifest_path, content),
        None if manifest_path.exists() => std::fs::remove_file(manifest_path),
        None => Ok(()),
    };
    if let Err(error) = manifest_restore {
        rollback_errors.push(format!(
            "could not restore {}: {error}",
            manifest_path.display()
        ));
    }
    for copied_source in copied_sources.iter().rev().filter(|path| path.exists()) {
        if let Err(error) = std::fs::remove_file(copied_source) {
            rollback_errors.push(format!(
                "could not remove copied source {}: {error}",
                copied_source.display()
            ));
        }
    }
    for cooked in copied_cooked.iter().rev().filter(|path| path.exists()) {
        if let Err(error) = std::fs::remove_file(cooked) {
            rollback_errors.push(format!(
                "could not remove copied cooked artifact {}: {error}",
                cooked.display()
            ));
        }
    }
    if rollback_errors.is_empty() {
        failure
    } else {
        format!(
            "{failure}\nasset import rollback also failed:\n{}",
            rollback_errors.join("\n")
        )
    }
}

pub(crate) fn cleanup_import_files(paths: &[PathBuf]) {
    for path in paths.iter().rev().filter(|path| path.exists()) {
        let _ = std::fs::remove_file(path);
    }
}
