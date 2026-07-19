//! Shared environment-capability probes for the sandbox test suites.
//!
//! Policy (ENG-71):
//!
//! - Tests that only need *some* child process must spawn an in-repo helper
//!   (the current test executable re-invoked in a helper mode), never a
//!   system tool. That keeps them hermetic on machines without PowerShell,
//!   a POSIX shell, or any other host binary.
//! - Tests that genuinely exercise a system tool (`dotnet`, PowerShell,
//!   shader compilers) must probe for it with [`require_tool`] and return
//!   early when it is missing. A machine without the tool then reports a
//!   clear capability SKIP instead of a fake logic failure.
//!
//! This file is shared by the integration tests (`mod common;`) and by the
//! crate's unit tests (via `#[path]` in `main.rs`), so it must stay
//! dependency-free (std only).

use std::path::Path;

/// Marker printed when a test skips because the environment lacks a tool.
/// CI logs can grep for this prefix to distinguish capability skips from
/// genuine failures.
pub const SKIP_MARKER: &str = "SKIP (missing capability)";

/// Probe whether `tool` resolves to an executable file. `tool` may be an
/// explicit path (used as-is) or a bare name searched through `PATH`,
/// honoring `PATHEXT` on Windows. The probe never spawns the tool.
pub fn tool_on_path(tool: &str) -> bool {
    let candidate = Path::new(tool);
    if candidate.components().count() > 1 {
        return is_executable_file(candidate);
    }
    let Some(paths) = std::env::var_os("PATH") else {
        return false;
    };
    executable_names(tool).into_iter().any(|name| {
        std::env::split_paths(&paths)
            .map(|directory| directory.join(&name))
            .any(|path| is_executable_file(&path))
    })
}

/// Guard for environment-coupled tests. Returns `true` when `tool` is
/// available; otherwise prints a [`SKIP_MARKER`] line and returns `false`
/// so the caller can early-return without failing:
///
/// ```ignore
/// #[test]
/// fn managed_roundtrip() {
///     if !common::require_tool("dotnet") {
///         return;
///     }
///     // ... exercise the real dotnet toolchain ...
/// }
/// ```
pub fn require_tool(tool: &str) -> bool {
    if tool_on_path(tool) {
        return true;
    }
    eprintln!("{SKIP_MARKER}: required tool `{tool}` was not found on PATH; skipping test");
    false
}

/// Candidate file names for a bare tool name: on Windows every `PATHEXT`
/// extension is tried, elsewhere the name is used verbatim.
fn executable_names(tool: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        if Path::new(tool).extension().is_some() {
            return vec![tool.to_string()];
        }
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".into());
        pathext
            .split(';')
            .filter(|extension| !extension.is_empty())
            .map(|extension| format!("{tool}{}", extension.to_ascii_lowercase()))
            .collect()
    }
    #[cfg(not(windows))]
    {
        vec![tool.to_string()]
    }
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return path
            .metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false);
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_missing_path_is_unavailable() {
        assert!(!tool_on_path("definitely/missing/tool-name"));
    }

    #[test]
    fn require_tool_skips_a_missing_tool() {
        assert!(!require_tool("definitely-missing-tool-eng71"));
    }

    #[cfg(windows)]
    #[test]
    fn pathext_lookup_finds_system_binary() {
        // `where` ships with Windows itself; if even that is absent the
        // runner is not a supported Windows environment.
        assert_eq!(
            tool_on_path("where"),
            tool_on_path("where.exe"),
            "PATHEXT resolution should agree with explicit-extension lookup"
        );
    }

    #[cfg(not(windows))]
    #[test]
    fn path_lookup_finds_sh() {
        assert!(tool_on_path("sh"));
    }
}
