//! C# build command integration.
//!
//! Provides [`build_csharp_project`] which shells out to `dotnet build`,
//! captures stdout/stderr, parses error lines, and returns the compiled
//! assembly path on success.

use std::path::{Path, PathBuf};
use std::process::Command;

use engine_serialize::{Diagnostic, DiagnosticSeverity};

// ---------------------------------------------------------------------------
// BuildResult
// ---------------------------------------------------------------------------

/// The outcome of a C# project build.
#[derive(Clone, Debug)]
pub struct BuildResult {
    /// Whether the build completed successfully.
    pub success: bool,
    /// The full stdout/stderr output from the build command.
    pub output: String,
    /// Parsed error lines (MSBuild-style).
    pub errors: Vec<String>,
    /// Path to the compiled assembly (DLL), if the build produced one.
    pub assembly_path: Option<PathBuf>,
}

impl BuildResult {
    /// Convert the build result into a list of editor [`Diagnostic`]s.
    ///
    /// *   If the build succeeded, a single info diagnostic is produced.
    /// *   If the build failed, each error line becomes an error diagnostic.
    /// *   The assembly path (if any) is attached as the diagnostic path.
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        if self.success {
            let mut diag = Diagnostic::new(
                "CSBUILD_OK",
                DiagnosticSeverity::Info,
                "build",
                format!(
                    "C# build succeeded{}",
                    self.assembly_path
                        .as_ref()
                        .map(|p| format!(" → {}", p.display()))
                        .unwrap_or_default()
                ),
            );
            if let Some(ref path) = self.assembly_path {
                diag = diag.path(path.to_string_lossy());
            }
            diagnostics.push(diag);
        } else {
            for error in &self.errors {
                // Try to extract a file path from MSBuild-style error lines:
                //   file(line,col): error CODE: message
                let path = extract_error_path(error);
                let mut diag = Diagnostic::new(
                    "CSBUILD_ERROR",
                    DiagnosticSeverity::Error,
                    "build",
                    error.clone(),
                );
                if let Some(p) = path {
                    diag = diag.path(p);
                }
                diagnostics.push(diag);
            }

            // Also attach the full output as a single diagnostic for
            // the "Diagnostics" panel.
            let mut summary = Diagnostic::new(
                "CSBUILD_FAILED",
                DiagnosticSeverity::Error,
                "build",
                format!(
                    "C# build failed with {} error(s)",
                    self.errors.len()
                ),
            );
            if let Some(ref path) = self.assembly_path {
                summary = summary.path(path.to_string_lossy());
            }
            diagnostics.push(summary);
        }

        diagnostics
    }
}

// ---------------------------------------------------------------------------
// BuildError
// ---------------------------------------------------------------------------

/// Errors that can occur while attempting a C# build.
#[derive(Clone, Debug)]
pub enum BuildError {
    /// The `dotnet` CLI was not found on `PATH`.
    DotnetNotFound,
    /// The build command itself failed (non-zero exit code without
    /// structured error lines).
    BuildFailed(String),
    /// An I/O or system error occurred.
    IoError(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::DotnetNotFound => {
                write!(f, "dotnet CLI not found on PATH")
            }
            BuildError::BuildFailed(msg) => {
                write!(f, "build failed: {msg}")
            }
            BuildError::IoError(msg) => {
                write!(f, "I/O error: {msg}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

// ---------------------------------------------------------------------------
// build_csharp_project
// ---------------------------------------------------------------------------

/// Build a C# project at `project_dir` using `dotnet build`.
///
/// The directory must contain a `.csproj` file.  This function shells out
/// to the `dotnet` CLI, captures all output, and returns a [`BuildResult`]
/// with the parsed error lines and the path to the compiled assembly.
pub fn build_csharp_project(project_dir: &Path) -> Result<BuildResult, BuildError> {
    // Resolve the project directory to an absolute path.
    let abs_project_dir = project_dir
        .canonicalize()
        .map_err(|e| BuildError::IoError(format!("cannot resolve project directory: {e}")))?;

    if !abs_project_dir.is_dir() {
        return Err(BuildError::IoError(format!(
            "not a directory: {}",
            abs_project_dir.display()
        )));
    }

    // Find the .csproj file (take the first one).
    let csproj = find_csproj(&abs_project_dir)?;

    // Locate `dotnet` on PATH.
    let dotnet = find_dotnet()?;

    // Run `dotnet build`.
    let output = Command::new(&dotnet)
        .arg("build")
        .arg(&csproj)
        .arg("--nologo")
        .current_dir(&abs_project_dir)
        .output()
        .map_err(|e| BuildError::IoError(format!("failed to execute dotnet: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let combined = if stderr.is_empty() {
        stdout.clone()
    } else {
        format!("{stdout}\n{stderr}")
    };

    let success = output.status.success();
    let errors = parse_error_lines(&combined);

    // Try to locate the built assembly (typically in
    // `bin/Debug/net*/`).
    let assembly_path = if success {
        find_assembly(&abs_project_dir)
    } else {
        None
    };

    Ok(BuildResult {
        success,
        output: combined,
        errors,
        assembly_path,
    })
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Locate the `dotnet` executable on `PATH`.
fn find_dotnet() -> Result<String, BuildError> {
    // Common names for the dotnet CLI on different platforms.
    for name in &["dotnet", "dotnet.exe"] {
        if let Ok(path) = which(name) {
            return Ok(path);
        }
    }
    Err(BuildError::DotnetNotFound)
}

/// Simple `which`-alike: search PATH for an executable.
fn which(name: &str) -> Result<String, ()> {
    let path_env = std::env::var_os("PATH").ok_or(())?;
    for dir in std::env::split_paths(&path_env) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            // On Windows also check with .exe appended.
            #[cfg(target_os = "windows")]
            {
                if candidate.extension().map_or(false, |ext| ext == "exe") {
                    return Ok(candidate.to_string_lossy().to_string());
                }
                let with_exe = candidate.with_extension("exe");
                if with_exe.is_file() {
                    return Ok(with_exe.to_string_lossy().to_string());
                }
            }
            #[cfg(not(target_os = "windows"))]
            {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }
    }
    Err(())
}

/// Find the first `.csproj` file in the given directory.
fn find_csproj(dir: &Path) -> Result<PathBuf, BuildError> {
    for entry in std::fs::read_dir(dir).map_err(|e| {
        BuildError::IoError(format!("cannot read directory {}: {e}", dir.display()))
    })? {
        let entry = entry.map_err(|e| {
            BuildError::IoError(format!("cannot read entry in {}: {e}", dir.display()))
        })?;
        let path = entry.path();
        if path.extension().map_or(false, |ext| ext == "csproj") {
            return Ok(path);
        }
    }
    Err(BuildError::IoError(format!(
        "no .csproj file found in {}",
        dir.display()
    )))
}

/// Locate the built assembly (DLL) in the project's output directory.
fn find_assembly(project_dir: &Path) -> Option<PathBuf> {
    // Look in bin/Debug/ for any .dll that matches the project name.
    let bin_debug = project_dir.join("bin").join("Debug");
    if !bin_debug.is_dir() {
        return None;
    }

    // Walk the target framework subdirectories (e.g. net8.0, net9.0).
    let mut found: Option<PathBuf> = None;
    if let Ok(entries) = std::fs::read_dir(&bin_debug) {
        for entry in entries.flatten() {
            let tfm_dir = entry.path();
            if !tfm_dir.is_dir() {
                continue;
            }
            // Look for .dll files (excluding system assemblies).
            if let Ok(files) = std::fs::read_dir(&tfm_dir) {
                for file in files.flatten() {
                    let path = file.path();
                    if path.extension().map_or(false, |ext| ext == "dll") {
                        // Skip known system assemblies.
                        let name = path
                            .file_stem()
                            .map(|s| s.to_string_lossy())
                            .unwrap_or_default();
                        if !name.starts_with("System")
                            && !name.starts_with("Microsoft")
                            && name != "mscorlib"
                        {
                            found = Some(path);
                        }
                    }
                }
            }
        }
    }
    found
}

/// Parse MSBuild-style error lines from build output.
///
/// Typical format:
/// ```text
/// file(line,col): error CODE: message
/// ```
fn parse_error_lines(output: &str) -> Vec<String> {
    let mut errors = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        // Match MSBuild error pattern: contains " : error "
        if trimmed.contains(" : error ") {
            errors.push(trimmed.to_string());
        }
        // Also catch lines starting with "error" (e.g. from dotnet CLI)
        if trimmed.starts_with("error ") || trimmed.starts_with("error:") {
            if !errors.iter().any(|e| e == trimmed) {
                errors.push(trimmed.to_string());
            }
        }
    }

    errors
}

/// Try to extract a file path from an MSBuild-style error line.
///
/// Format: `file(line,col): error CODE: message`
fn extract_error_path(line: &str) -> Option<String> {
    // Find the first '(' which indicates the start of line/col info.
    if let Some(paren) = line.find('(') {
        let path_part = &line[..paren];
        if !path_part.is_empty() && !path_part.contains(' ') {
            return Some(path_part.to_string());
        }
    }
    // Also try looking for a path before " : error "
    if let Some(pos) = line.find(" : error ") {
        let before = &line[..pos];
        // If it looks like a file path (contains a dot or slash)
        if before.contains('.') || before.contains('\\') || before.contains('/') {
            return Some(before.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_error_dotnet_not_found_display() {
        let err = BuildError::DotnetNotFound;
        assert_eq!(err.to_string(), "dotnet CLI not found on PATH");
    }

    #[test]
    fn build_error_build_failed_display() {
        let err = BuildError::BuildFailed("syntax error".to_string());
        assert_eq!(err.to_string(), "build failed: syntax error");
    }

    #[test]
    fn build_error_io_error_display() {
        let err = BuildError::IoError("permission denied".to_string());
        assert_eq!(err.to_string(), "I/O error: permission denied");
    }

    #[test]
    fn build_result_success_to_diagnostics() {
        let result = BuildResult {
            success: true,
            output: "Build succeeded.".to_string(),
            errors: vec![],
            assembly_path: Some(PathBuf::from("bin/Debug/net8.0/MyProject.dll")),
        };
        let diags = result.to_diagnostics();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Info);
        assert_eq!(diags[0].code, "CSBUILD_OK");
    }

    #[test]
    fn build_result_failure_to_diagnostics() {
        let result = BuildResult {
            success: false,
            output: "Build FAILED.".to_string(),
            errors: vec![
                "src/Program.cs(10,5): error CS1001: Identifier expected".to_string(),
            ],
            assembly_path: None,
        };
        let diags = result.to_diagnostics();
        // One error diagnostic per error line + one summary
        assert_eq!(diags.len(), 2);
        assert_eq!(diags[0].severity, DiagnosticSeverity::Error);
        assert_eq!(diags[0].code, "CSBUILD_ERROR");
        assert_eq!(diags[1].code, "CSBUILD_FAILED");
    }

    #[test]
    fn build_result_assembly_path_in_diagnostics() {
        let result = BuildResult {
            success: true,
            output: "OK".to_string(),
            errors: vec![],
            assembly_path: Some(PathBuf::from("out/output.dll")),
        };
        let diags = result.to_diagnostics();
        assert_eq!(diags[0].path.as_deref(), Some("out/output.dll"));
    }

    #[test]
    fn parse_error_lines_empty() {
        assert!(parse_error_lines("").is_empty());
        assert!(parse_error_lines("Build succeeded.\nNo errors.").is_empty());
    }

    #[test]
    fn parse_error_lines_msbuild_style() {
        let output = "\
Build started...
src/Program.cs(10,5): error CS1001: Identifier expected [project.csproj]
    1 Warning(s)
    1 Error(s)
";
        let errors = parse_error_lines(output);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("error CS1001"));
    }

    #[test]
    fn parse_error_lines_dotnet_cli_style() {
        let output = "\
error MSB4025: The project file could not be loaded.
Build FAILED.
";
        let errors = parse_error_lines(output);
        assert!(!errors.is_empty());
        assert!(errors[0].contains("error MSB4025"));
    }

    #[test]
    fn extract_error_path_with_parens() {
        let line = "src/Program.cs(10,5): error CS1001: Identifier expected";
        assert_eq!(extract_error_path(line), Some("src/Program.cs".to_string()));
    }

    #[test]
    fn extract_error_path_with_colon() {
        let line = "src/Program.cs : error CS1001: Identifier expected";
        assert_eq!(extract_error_path(line), Some("src/Program.cs".to_string()));
    }

    #[test]
    fn extract_error_path_no_match() {
        assert_eq!(extract_error_path("error CS1001: general error"), None);
    }

    #[test]
    fn find_csprof_nonexistent_dir() {
        let result = find_csproj(Path::new("/nonexistent/path"));
        assert!(result.is_err());
        match result {
            Err(BuildError::IoError(_)) => {} // expected
            _ => panic!("expected IoError"),
        }
    }

    #[test]
    fn build_error_is_error() {
        use std::error::Error;
        let err = BuildError::DotnetNotFound;
        assert!(err.source().is_none());
    }
}
