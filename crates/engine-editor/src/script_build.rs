//! C# script compilation via `dotnet build`.
//!
//! [`ScriptBuildManager`] wraps a .NET project directory and provides
//! one-shot invocation of `dotnet build`, capturing stdout/stderr and
//! extracting compiler error messages for the editor diagnostics system.

use std::path::PathBuf;
use std::process::Command;
use std::time::Instant;

use crate::EditorError;

// -------------------------------------------------------------------
// BuildResult
// -------------------------------------------------------------------

/// Outcome of a C# script compilation run.
#[derive(Clone, Debug)]
pub struct BuildResult {
    /// Whether the build completed with exit code 0.
    pub success: bool,
    /// Full raw output (stdout + stderr merged).
    pub output: String,
    /// Individual error lines parsed from the output.
    pub errors: Vec<String>,
    /// Wall-clock time the build finished.
    pub compiled_at: Instant,
}

// -------------------------------------------------------------------
// ScriptBuildManager
// -------------------------------------------------------------------

/// Manages C# script compilation for the editor.
///
/// Holds a reference to the .NET project directory and caches the most
/// recent build result.
pub struct ScriptBuildManager {
    /// Path to the directory containing the `.csproj` file.
    project_dir: PathBuf,
    /// Cached result of the last build invocation.
    last_build_result: Option<BuildResult>,
}

impl ScriptBuildManager {
    /// Create a new build manager rooted at `project_dir`.
    ///
    /// `project_dir` should be the directory containing a `.csproj` file.
    /// No validation is performed at construction time.
    pub fn new(project_dir: PathBuf) -> Self {
        Self {
            project_dir,
            last_build_result: None,
        }
    }

    /// Run `dotnet build` on the configured project directory.
    ///
    /// Captures stdout and stderr, parses the output for error lines
    /// (containing `error CS` or `error :`), and stores the result.
    ///
    /// Returns `Ok(())` even when the build fails — the caller should
    /// inspect [`last_result`](Self::last_result) for the actual outcome.
    pub fn build(&mut self) -> Result<(), EditorError> {
        let start = Instant::now();

        let output = Command::new("dotnet")
            .arg("build")
            .arg(&self.project_dir)
            .output()
            .map_err(|e| EditorError::IoFailed(format!("failed to launch dotnet build: {e}")))?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        let merged = if stderr.is_empty() {
            stdout
        } else {
            format!("{stdout}\n{stderr}")
        };

        // Parse error lines: "error CS..." or "error :..."
        let errors: Vec<String> = merged
            .lines()
            .filter(|line| {
                let lower = line.to_lowercase();
                lower.contains("error cs") || lower.contains("error :")
            })
            .map(|s| s.to_string())
            .collect();

        let success = output.status.success();

        self.last_build_result = Some(BuildResult {
            success,
            output: merged,
            errors,
            compiled_at: start,
        });

        tracing::info!(
            success = success,
            error_count = self
                .last_build_result
                .as_ref()
                .map(|r| r.errors.len())
                .unwrap_or(0),
            elapsed_ms = start.elapsed().as_millis(),
            "ScriptBuildManager::build completed"
        );

        Ok(())
    }

    /// Return a reference to the most recent build result, if any.
    pub fn last_result(&self) -> Option<&BuildResult> {
        self.last_build_result.as_ref()
    }

    /// The configured project directory.
    pub fn project_dir(&self) -> &PathBuf {
        &self.project_dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_build_manager_new() {
        let mgr = ScriptBuildManager::new(PathBuf::from("/tmp/test_project"));
        assert!(mgr.last_result().is_none());
        assert_eq!(mgr.project_dir(), &PathBuf::from("/tmp/test_project"));
    }

    #[test]
    fn script_build_result_fields() {
        let result = BuildResult {
            success: true,
            output: "Build succeeded.".to_string(),
            errors: Vec::new(),
            compiled_at: Instant::now(),
        };
        assert!(result.success);
        assert!(result.errors.is_empty());
        assert_eq!(result.output, "Build succeeded.");
    }

    #[test]
    fn script_build_manager_build_no_dotnet() {
        // Without `dotnet` on PATH, this should return an IoFailed error.
        // We simulate by pointing to a non-existent directory.
        let mut mgr = ScriptBuildManager::new(PathBuf::from("Z:\\nonexistent\\project"));
        let result = mgr.build();
        // The result may be Ok (if dotnet happens to be installed) or Err.
        // If it's Ok, the build likely failed with a non-zero exit (which
        // is still Ok from the manager's perspective).
        if result.is_ok() {
            let r = mgr.last_result().unwrap();
            // On a non-existent project dotnet will fail.
            assert!(!r.success);
            assert!(!r.errors.is_empty() || !r.output.is_empty());
        } else {
            assert!(result.is_err());
        }
    }
}
