use super::*;

pub(crate) fn create_project_script_in(
    project: &GameProject,
    relative_folder: &Path,
    class_name: &str,
) -> Result<PathBuf, String> {
    let class_name = class_name.trim();
    let mut characters = class_name.chars();
    let valid_first = characters
        .next()
        .is_some_and(|character| character == '_' || character.is_ascii_alphabetic());
    if !valid_first
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(format!(
            "script class name must be a C# identifier: '{class_name}'"
        ));
    }
    let script_project = project
        .script_project
        .as_ref()
        .ok_or_else(|| "project has no C# script project configured".to_string())?;
    let source_directory = script_project.parent().ok_or_else(|| {
        format!(
            "script project has no source directory: {}",
            script_project.display()
        )
    })?;
    let relative_folder = portable_script_subdirectory(relative_folder)?;
    let source_directory = ensure_script_source_directory(source_directory, &relative_folder)?;
    let path = source_directory.join(format!("{class_name}.cs"));
    let source = format!(
        "using Engine;\n\nnamespace GameScripts;\n\npublic sealed class {class_name} : EngineBehaviour\n{{\n    public void OnCreate()\n    {{\n    }}\n\n    public void OnStart()\n    {{\n    }}\n\n    public void OnUpdate(float deltaTime)\n    {{\n    }}\n}}\n"
    );
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|error| format!("could not create script {}: {error}", path.display()))?;
    std::io::Write::write_all(&mut file, source.as_bytes())
        .map_err(|error| format!("could not write script {}: {error}", path.display()))?;
    file.sync_all()
        .map_err(|error| format!("could not flush script {}: {error}", path.display()))?;
    Ok(path)
}

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) fn portable_script_subdirectory(path: &Path) -> Result<PathBuf, String> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(format!(
                "script folder must be a safe project-relative path: {}",
                path.display()
            ));
        };
        let component = component
            .to_str()
            .ok_or_else(|| "script folder contains non-UTF-8 text".to_string())?;
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
                "script folder contains a non-portable component '{component}'"
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
                "script folder uses reserved portable name '{component}'"
            ));
        }
        normalized.push(component);
    }
    Ok(normalized)
}

#[cfg(any(feature = "tooling-editor", test))]
pub(crate) fn ensure_script_source_directory(
    root: &Path,
    relative: &Path,
) -> Result<PathBuf, String> {
    let metadata = std::fs::symlink_metadata(root).map_err(|error| {
        format!(
            "could not inspect script source {}: {error}",
            root.display()
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(format!(
            "script source is not a real directory: {}",
            root.display()
        ));
    }
    let mut current = root.to_path_buf();
    for component in relative.components() {
        let std::path::Component::Normal(component) = component else {
            return Err(format!(
                "script folder is not normalized: {}",
                relative.display()
            ));
        };
        current.push(component);
        if current.exists() {
            let metadata = std::fs::symlink_metadata(&current).map_err(|error| {
                format!(
                    "could not inspect script folder {}: {error}",
                    current.display()
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(format!(
                    "script folder is not a real directory: {}",
                    current.display()
                ));
            }
        } else {
            std::fs::create_dir(&current).map_err(|error| {
                format!(
                    "could not create script folder {}: {error}",
                    current.display()
                )
            })?;
        }
    }
    Ok(current)
}
