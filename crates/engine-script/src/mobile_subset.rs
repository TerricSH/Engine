//! Documented mobile-safe subset of ScriptAPI-v0.
//!
//! This module defines which .NET/C# ScriptAPI features are available on
//! mobile (AOT) runtimes and which are desktop-only (CoreCLR with JIT).
//! It is a **data-only** declaration — consumers (hot-reload, editor UI,
//! asset pipeline) use it to present warnings or block deployment when a
//! script uses an unsupported API.
//!
//! # Versioning
//!
//! The mobile-safe subset is tied to a specific version range of the
//! ScriptAPI schema.  The [`mobile_subset_v0`] function returns the
//! constraints for **ScriptAPI-v0** (which encompasses all 0.x releases).
//!
//! # Related modules
//!
//! * [`crate::api_compat`] — [`ApiCompatRange`] and the
//!   [`MOBILE_SAFE_FEATURES`][crate::api_compat::MOBILE_SAFE_FEATURES] /
//!   [`DESKTOP_ONLY_FEATURES`][crate::api_compat::DESKTOP_ONLY_FEATURES]
//!   string constants used by the constraint system.
//! * [`crate::profile`] — [`PlatformProfile`][crate::profile::PlatformProfile]
//!   enum and runtime constraint validation.

/// A versioned declaration of which ScriptAPI features are available on
/// mobile (AOT-compiled) platforms.
///
/// The range `[min_version, max_version]` identifies the schema version of
/// ScriptAPI to which this subset applies.  Any script assembly targeting
/// a version outside this range should be checked against a different
/// subset declaration.
///
/// # Examples
///
/// ```
/// use engine_script::mobile_subset::mobile_subset_v0;
///
/// let subset = mobile_subset_v0();
/// assert_eq!(subset.min_version, "0.1.0");
///
/// // Every unsupported pattern has a name and description.
/// for pat in &subset.unsupported_patterns {
///     assert!(!pat.name.is_empty());
///     assert!(!pat.description.is_empty());
/// }
/// ```
#[derive(Clone, Debug)]
pub struct ScriptApiSubset {
    /// Minimum inclusive schema version of ScriptAPI covered by this subset
    /// (semver string, e.g. `"0.1.0"`).
    pub min_version: String,
    /// Maximum inclusive schema version of ScriptAPI covered by this subset
    /// (semver string, e.g. `"0.999.0"`).
    pub max_version: String,
    /// List of API patterns that are **not** available on mobile runtimes.
    pub unsupported_patterns: Vec<UnsupportedPattern>,
}

/// A single API pattern that is unsupported on mobile (AOT) runtimes.
///
/// Each entry includes a human-readable description, an example of the
/// problematic code, and — where possible — a mobile-safe alternative.
#[derive(Clone, Debug)]
pub struct UnsupportedPattern {
    /// Short identifier for the pattern (e.g. `"Assembly.LoadFrom"`).
    pub name: &'static str,
    /// Human-readable explanation of why this pattern is unsupported.
    pub description: &'static str,
    /// Example code that would be rejected (optional).
    pub example: Option<&'static str>,
    /// Suggested alternative that works on mobile runtimes (optional).
    pub mobile_alternative: Option<&'static str>,
}

/// Return the documented mobile-safe subset for **ScriptAPI-v0**.
///
/// This covers the initial ScriptAPI schema (versions `0.1.0` through
/// `0.999.0`).  The returned [`ScriptApiSubset`] lists every API pattern
/// that is known to fail under NativeAOT or restricted Android runtimes.
///
/// # Unsupported Patterns Summary
///
/// | Pattern | Desktop | Android | iOS |
/// |---------|---------|---------|-----|
/// | `Assembly.LoadFrom` / `LoadFile` | ✅ | ❌ | ❌ |
/// | `System.Reflection.Emit.ILGenerator` | ✅ | ❌ | ❌ |
/// | `Type.MakeGenericType` (open generics) | ✅ | ⚠️ limited | ❌ |
/// | `Activator.CreateInstance` (non-AOT) | ✅ | ⚠️ limited | ❌ |
/// | `Marshal.GetFunctionPointerForDelegate` | ✅ | ❌ | ❌ |
/// | `DynamicMethod` | ✅ | ❌ | ❌ |
/// | `AssemblyBuilder` / `ModuleBuilder` | ✅ | ❌ | ❌ |
/// | `Microsoft.CSharp.CSharpCodeProvider` | ✅ | ❌ | ❌ |
/// | `System.Linq.Expressions.Expression.Lambda` (compiled) | ✅ | ⚠️ | ❌ |
/// | P/Invoke to undeclared native libraries | ✅ | ❌ | ❌ |
pub fn mobile_subset_v0() -> ScriptApiSubset {
    ScriptApiSubset {
        min_version: "0.1.0".to_string(),
        max_version: "0.999.0".to_string(),
        unsupported_patterns: vec![
            UnsupportedPattern {
                name: "Assembly.LoadFrom",
                description: "Loading assemblies from byte streams or file paths \
                    at runtime is not supported under NativeAOT because all \
                    managed code must be linked ahead of time.",
                example: Some("Assembly.LoadFrom(\"Mod.dll\")"),
                mobile_alternative: Some("Reference assemblies via the project file; \
                    use the engine's plugin registry for dynamic loads."),
            },
            UnsupportedPattern {
                name: "Reflection.Emit.ILGenerator",
                description: "Runtime IL emission requires JIT compilation, \
                    which is unavailable on iOS and restricted on Android.",
                example: Some("new DynamicMethod(\"Foo\", ...).GetILGenerator()"),
                mobile_alternative: Some("Use expression trees with AOT-friendly \
                    compilation, or pre-generate IL at build time."),
            },
            UnsupportedPattern {
                name: "Type.MakeGenericType (open generics)",
                description: "Constructing generic types over open generic \
                    parameters at runtime may fail under AOT because the \
                    runtime cannot specialise the generic at compile time.",
                example: Some("typeof(Dict<,>).MakeGenericType(typeof(int), typeof(string))"),
                mobile_alternative: Some("Use closed generic types directly. \
                    For dynamic dispatch, consider an interface-based factory."),
            },
            UnsupportedPattern {
                name: "Activator.CreateInstance (non-AOT)",
                description: "Creating instances of types discovered at runtime \
                    via string names (`Activator.CreateInstance(string, string)`) \
                    relies on reflection over unlinked assemblies.",
                example: Some("Activator.CreateInstance(\"ExternalAssembly\", \"MyType\")"),
                mobile_alternative: Some("Register known types via a static factory \
                    or dependency injection container."),
            },
            UnsupportedPattern {
                name: "Marshal.GetFunctionPointerForDelegate",
                description: "Converting a delegate to a native function pointer \
                    requires runtime code stubs that NativeAOT cannot generate.",
                example: Some("Marshal.GetFunctionPointerForDelegate(myDelegate)"),
                mobile_alternative: Some("Use [UnmanagedCallersOnly] for static \
                    callbacks or DllImport for native entry points."),
            },
            UnsupportedPattern {
                name: "DynamicMethod",
                description: "DynamicMethod creates lightweight methods at \
                    runtime, which depends on JIT compilation.",
                example: Some("var dm = new DynamicMethod(\"Handler\", ...);"),
                mobile_alternative: Some("Define methods statically in source code; \
                    use delegates for dynamic dispatch."),
            },
            UnsupportedPattern {
                name: "AssemblyBuilder / ModuleBuilder",
                description: "Generating entire assemblies at runtime requires \
                    both JIT and Reflection.Emit infrastructure not present \
                    in AOT runtimes.",
                example: Some("AssemblyBuilder.DefineDynamicAssembly(...)"),
                mobile_alternative: Some("Pre-compile all assemblies; load them \
                    via static project references."),
            },
            UnsupportedPattern {
                name: "CSharpCodeProvider",
                description: "Compiling C# source code at runtime requires \
                    the full Roslyn compiler and JIT, unavailable on mobile.",
                example: Some("new CSharpCodeProvider().CompileAssemblyFromSource(...)"),
                mobile_alternative: Some("Pre-compile scripts as part of the \
                    application build pipeline."),
            },
            UnsupportedPattern {
                name: "Expression<T>.Compile (LINQ Expressions)",
                description: "Compiling expression trees to IL at runtime \
                    requires JIT, which is absent on iOS.",
                example: Some("Expression<Func<int, int>> expr = x => x + 1;\nexpr.Compile()"),
                mobile_alternative: Some("Use interpreted Expression.Invoke or \
                    pre-compile expressions at build time with AOT-compatible tools."),
            },
            UnsupportedPattern {
                name: "P/Invoke undeclared native libraries",
                description: "NativeAOT requires all P/Invoke targets to be \
                    declared at compile time. Dynamically loading native \
                    libraries via DllImport with runtime-resolved paths fails.",
                example: Some("[DllImport(\"user-provided.dll\")]"),
                mobile_alternative: Some("Declare all native dependencies in the \
                    project file; use the engine's native-plugin API for dynamic loads."),
            },
        ],
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── ScriptApiSubset ───────────────────────────────────────────────────

    #[test]
    fn mobile_subset_v0_version_range() {
        let subset = mobile_subset_v0();
        assert_eq!(subset.min_version, "0.1.0");
        assert_eq!(subset.max_version, "0.999.0");
    }

    #[test]
    fn mobile_subset_v0_contains_expected_patterns() {
        let subset = mobile_subset_v0();
        let names: Vec<&str> = subset
            .unsupported_patterns
            .iter()
            .map(|p| p.name)
            .collect();

        assert!(names.contains(&"Assembly.LoadFrom"));
        assert!(names.contains(&"Reflection.Emit.ILGenerator"));
        assert!(names.contains(&"Type.MakeGenericType (open generics)"));
        assert!(names.contains(&"DynamicMethod"));
        assert!(names.contains(&"AssemblyBuilder / ModuleBuilder"));
    }

    #[test]
    fn mobile_subset_v0_assembly_load_from_has_alternative() {
        let subset = mobile_subset_v0();
        let pat = subset
            .unsupported_patterns
            .iter()
            .find(|p| p.name == "Assembly.LoadFrom")
            .expect("Assembly.LoadFrom pattern should exist");
        assert!(pat.mobile_alternative.is_some());
        assert!(pat.description.contains("NativeAOT"));
    }

    #[test]
    fn mobile_subset_v0_reflection_emit_has_example() {
        let subset = mobile_subset_v0();
        let pat = subset
            .unsupported_patterns
            .iter()
            .find(|p| p.name.starts_with("Reflection.Emit"))
            .expect("Reflection.Emit pattern should exist");
        assert!(pat.example.is_some());
        assert!(pat.description.contains("JIT"));
    }

    #[test]
    fn mobile_subset_v0_every_pattern_has_name_and_description() {
        let subset = mobile_subset_v0();
        for pat in &subset.unsupported_patterns {
            assert!(!pat.name.is_empty());
            assert!(!pat.description.is_empty());
        }
    }

    #[test]
    fn mobile_subset_v0_clone() {
        let a = mobile_subset_v0();
        let b = a.clone();
        assert_eq!(a.min_version, b.min_version);
        assert_eq!(a.max_version, b.max_version);
        assert_eq!(a.unsupported_patterns.len(), b.unsupported_patterns.len());
    }

    #[test]
    fn mobile_subset_v0_debug() {
        let subset = mobile_subset_v0();
        let debug = format!("{:?}", subset);
        assert!(debug.contains("ScriptApiSubset"));
        assert!(debug.contains("0.1.0"));
        assert!(debug.contains("0.999.0"));
    }

    #[test]
    fn mobile_subset_v0_pattern_count() {
        // Sanity: we should have a meaningful number of entries.
        let subset = mobile_subset_v0();
        assert!(
            subset.unsupported_patterns.len() >= 8,
            "expected at least 8 unsupported patterns, got {}",
            subset.unsupported_patterns.len()
        );
    }
}
