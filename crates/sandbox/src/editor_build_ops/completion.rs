impl CompletionPlan {
    pub(super) fn finish(
        self,
        elapsed: Duration,
        output: EditorBuildOutput,
    ) -> Result<EditorBuildResult, EditorBuildError> {
        match self {
            Self::Validate { report_path, .. } => finish_validation(&report_path, elapsed, output),
            Self::CookAndCompile { manifest_path } => {
                finish_cook_and_compile(&manifest_path, elapsed, output)
            }
            Self::PackageWindows {
                version,
                allow_dirty,
                release_root,
            } => finish_windows_package(version, allow_dirty, release_root, elapsed, output),
        }
    }
}

pub(super) fn finish_validation(
    report_path: &Path,
    elapsed: Duration,
    output: EditorBuildOutput,
) -> Result<EditorBuildResult, EditorBuildError> {
    let operation = EditorBuildOperationKind::Validate;
    let report = read_json(report_path).map_err(|message| {
        EditorBuildError::process(
            operation,
            EditorBuildFailureKind::InvalidResult,
            message,
            None,
            output.clone(),
        )
    })?;
    require_json_string(&report, "schema", PROJECT_CHECK_SCHEMA)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    if report.get("passed").and_then(Value::as_bool) != Some(true) {
        return Err(invalid_result(
            operation,
            "project check report does not say passed=true",
            output,
        ));
    }
    let project = required_string(&report, "project")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let startup_scene_id = required_string(&report, "startup_scene_id")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let scenes = required_u64(&report, "scenes")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let entities = required_u64(&report, "entities")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let declared_assets = required_u64(&report, "declared_assets")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let cooked_assets = required_u64(&report, "cooked_assets")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;

    Ok(EditorBuildResult::Validated(ProjectValidationResult {
        project,
        startup_scene_id,
        scenes,
        entities,
        declared_assets,
        cooked_assets,
        report,
        elapsed,
        output,
    }))
}

fn finish_cook_and_compile(
    manifest_path: &Path,
    elapsed: Duration,
    output: EditorBuildOutput,
) -> Result<EditorBuildResult, EditorBuildError> {
    let operation = EditorBuildOperationKind::CookAndCompile;
    let project = GameProject::load(manifest_path).map_err(|error| {
        invalid_result(
            operation,
            format!("built project can no longer be loaded: {error}"),
            output.clone(),
        )
    })?;
    if !project.cooked_assets.is_dir() {
        return Err(invalid_result(
            operation,
            format!(
                "cook command succeeded without producing {}",
                project.cooked_assets.display()
            ),
            output,
        ));
    }
    Ok(EditorBuildResult::CookedAndCompiled(CookAndCompileResult {
        project: project.manifest.name,
        manifest_path: project.manifest_path,
        cooked_assets: project.cooked_assets,
        scripts_configured: project.script_project.is_some(),
        elapsed,
        output,
    }))
}

pub(super) fn finish_windows_package(
    version: String,
    allow_dirty: bool,
    release_root: PathBuf,
    elapsed: Duration,
    output: EditorBuildOutput,
) -> Result<EditorBuildResult, EditorBuildError> {
    let operation = EditorBuildOperationKind::PackageWindows;
    let stage_root = release_root.join(WINDOWS_PLATFORM);
    let release_manifest_path = stage_root.join("manifests/release.json");
    let release_manifest = read_json(&release_manifest_path)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "schema", RELEASE_METADATA_SCHEMA)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "release_id", &version)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "platform", WINDOWS_PLATFORM)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    require_json_string(&release_manifest, "backend", "vulkan")
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let dirty = release_manifest
        .get("dirty")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            invalid_result(
                operation,
                "release metadata field 'dirty' is missing or is not a boolean",
                output.clone(),
            )
        })?;
    if dirty && !allow_dirty {
        return Err(invalid_result(
            operation,
            "release metadata is dirty although dirty packaging was not authorized",
            output,
        ));
    }

    let archive_path = release_root.join(format!("{WINDOWS_PLATFORM}.zip"));
    let symbols_archive_path = release_root.join(format!("{WINDOWS_PLATFORM}-symbols.zip"));
    let archive_sha256 = verify_checksum_sidecar(&archive_path)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;
    let symbols_sha256 = verify_checksum_sidecar(&symbols_archive_path)
        .map_err(|message| invalid_result(operation, message, output.clone()))?;

    Ok(EditorBuildResult::PackagedWindows(PackageWindowsResult {
        version,
        release_root,
        archive_path,
        archive_sha256,
        symbols_archive_path,
        symbols_sha256,
        release_manifest_path,
        dirty,
        elapsed,
        output,
    }))
}

fn invalid_result(
    operation: EditorBuildOperationKind,
    message: impl Into<String>,
    output: EditorBuildOutput,
) -> EditorBuildError {
    EditorBuildError::process(
        operation,
        EditorBuildFailureKind::InvalidResult,
        message,
        None,
        output,
    )
}
use super::*;
