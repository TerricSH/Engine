//! # Script API Extension Surface
//!
//! Provides a plugin-style registration interface for extending the C# script
//! API with user-defined types and methods.  Third-party crates can implement
//! [`ScriptApiProvider`] and register it on the
//! [`ScriptApiExtensionRegistry`] to inject additional bindings.

// ---------------------------------------------------------------------------
// ScriptApiExtensionMeta
// ---------------------------------------------------------------------------

/// Metadata for a script API extension.
#[derive(Clone, Debug)]
pub struct ScriptApiExtensionMeta {
    /// Human-readable extension name.
    pub name: &'static str,
    /// Extension version string (semver recommended).
    pub version: &'static str,
    /// Minimum and maximum supported ScriptAPI-v0 minor version.
    ///
    /// The first element is the minimum supported minor version, the second is
    /// the maximum.  For example, `(0, 5)` means this extension works with
    /// ScriptAPI v0.0 through v0.5.
    pub api_version_range: (u16, u16),
    /// Names of other extensions this extension depends on.
    pub dependencies: Vec<&'static str>,
}

// ---------------------------------------------------------------------------
// ProvidedType / ProvidedMethod
// ---------------------------------------------------------------------------

/// A type exposed to C# scripts.
///
/// Describes a single type (class or struct) that should be generated or
/// reflected into the C# script API surface.
#[derive(Clone, Debug)]
pub struct ProvidedType {
    /// Fully qualified type name (e.g. `"Engine.Math.Vector3"`).
    pub full_name: &'static str,
    /// Optional base type name (e.g. `"System.ValueType"`).
    pub base_type: Option<&'static str>,
    /// Names of fields/properties exposed on this type.
    pub fields: Vec<&'static str>,
}

/// A method exposed to C# scripts.
///
/// Describes a single method belonging to a [`ProvidedType`].
#[derive(Clone, Debug)]
pub struct ProvidedMethod {
    /// The owning type's fully qualified name.
    pub type_name: &'static str,
    /// The method name as it should appear in C#.
    pub method_name: &'static str,
    /// Parameter type names in declaration order.
    pub parameters: Vec<&'static str>,
    /// Return type name (e.g. `"void"`, `"System.Single"`).
    pub return_type: &'static str,
}

// ---------------------------------------------------------------------------
// ScriptApiProvider trait
// ---------------------------------------------------------------------------

/// A binding provider that adds C# types/methods to the script API.
///
/// Implementations describe the API surface they contribute.  Multiple
/// providers can be registered on a [`ScriptApiExtensionRegistry`].
pub trait ScriptApiProvider: Send {
    /// Metadata describing this provider.
    fn meta(&self) -> &ScriptApiExtensionMeta;

    /// The types this provider wishes to expose.
    fn provide_types(&self) -> Vec<ProvidedType>;

    /// The methods this provider wishes to expose.
    fn provide_methods(&self) -> Vec<ProvidedMethod>;
}

// ---------------------------------------------------------------------------
// ScriptApiExtensionRegistry
// ---------------------------------------------------------------------------

/// Registry for script API extensions.
///
/// Collects [`ScriptApiProvider`]s and provides aggregated views of all
/// registered types and methods.  This is the primary integration point for
/// extending the C# script API from Rust crates.
pub struct ScriptApiExtensionRegistry {
    providers: Vec<Box<dyn ScriptApiProvider>>,
}

impl ScriptApiExtensionRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    /// Register a [`ScriptApiProvider`].
    pub fn register(&mut self, provider: Box<dyn ScriptApiProvider>) {
        self.providers.push(provider);
    }

    /// Aggregate all types from all registered providers.
    pub fn all_types(&self) -> Vec<ProvidedType> {
        let mut types = Vec::new();
        for provider in &self.providers {
            types.extend(provider.provide_types());
        }
        types
    }

    /// Aggregate all methods from all registered providers.
    pub fn all_methods(&self) -> Vec<ProvidedMethod> {
        let mut methods = Vec::new();
        for provider in &self.providers {
            methods.extend(provider.provide_methods());
        }
        methods
    }

    /// The number of registered providers.
    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }
}

impl Default for ScriptApiExtensionRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── Dummy provider for testing ───────────────────────────────────────

    struct DummyProvider;

    impl ScriptApiProvider for DummyProvider {
        fn meta(&self) -> &ScriptApiExtensionMeta {
            // Lazy-static-like pattern – initialise once via `once_cell` isn't
            // available, so we return a reference to a static that is
            // populated at compile time with an empty vec.
            static META: ScriptApiExtensionMeta = ScriptApiExtensionMeta {
                name: "dummy",
                version: "1.0.0",
                api_version_range: (0, 5),
                dependencies: Vec::new(),
            };
            &META
        }

        fn provide_types(&self) -> Vec<ProvidedType> {
            vec![ProvidedType {
                full_name: "Engine.Dummy.MyType",
                base_type: Some("System.Object"),
                fields: vec!["Value"],
            }]
        }

        fn provide_methods(&self) -> Vec<ProvidedMethod> {
            vec![ProvidedMethod {
                type_name: "Engine.Dummy.MyType",
                method_name: "GetValue",
                parameters: vec![],
                return_type: "System.Int32",
            }]
        }
    }

    // ── Registry tests ───────────────────────────────────────────────────

    #[test]
    fn registry_new_is_empty() {
        let reg = ScriptApiExtensionRegistry::new();
        assert_eq!(reg.provider_count(), 0);
    }

    #[test]
    fn registry_default_is_empty() {
        let reg = ScriptApiExtensionRegistry::default();
        assert_eq!(reg.provider_count(), 0);
    }

    #[test]
    fn registry_register_increases_count() {
        let mut reg = ScriptApiExtensionRegistry::new();
        reg.register(Box::new(DummyProvider));
        assert_eq!(reg.provider_count(), 1);
    }

    #[test]
    fn registry_register_multiple_providers() {
        let mut reg = ScriptApiExtensionRegistry::new();
        reg.register(Box::new(DummyProvider));
        reg.register(Box::new(DummyProvider));
        assert_eq!(reg.provider_count(), 2);
    }

    #[test]
    fn all_types_returns_registered_types() {
        let mut reg = ScriptApiExtensionRegistry::new();
        reg.register(Box::new(DummyProvider));
        let types = reg.all_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].full_name, "Engine.Dummy.MyType");
    }

    #[test]
    fn all_types_empty_when_no_providers() {
        let reg = ScriptApiExtensionRegistry::new();
        assert!(reg.all_types().is_empty());
    }

    #[test]
    fn all_types_aggregates_multiple_providers() {
        let mut reg = ScriptApiExtensionRegistry::new();
        reg.register(Box::new(DummyProvider));
        reg.register(Box::new(DummyProvider));
        assert_eq!(reg.all_types().len(), 2);
    }

    #[test]
    fn all_methods_returns_registered_methods() {
        let mut reg = ScriptApiExtensionRegistry::new();
        reg.register(Box::new(DummyProvider));
        let methods = reg.all_methods();
        assert_eq!(methods.len(), 1);
        assert_eq!(methods[0].method_name, "GetValue");
        assert_eq!(methods[0].type_name, "Engine.Dummy.MyType");
    }

    #[test]
    fn all_methods_empty_when_no_providers() {
        let reg = ScriptApiExtensionRegistry::new();
        assert!(reg.all_methods().is_empty());
    }

    #[test]
    fn all_methods_aggregates_multiple_providers() {
        let mut reg = ScriptApiExtensionRegistry::new();
        reg.register(Box::new(DummyProvider));
        reg.register(Box::new(DummyProvider));
        assert_eq!(reg.all_methods().len(), 2);
    }

    // ── Dummy provider tests ─────────────────────────────────────────────

    #[test]
    fn dummy_provider_meta() {
        let provider = DummyProvider;
        let meta = provider.meta();
        assert_eq!(meta.name, "dummy");
        assert_eq!(meta.version, "1.0.0");
        assert_eq!(meta.api_version_range, (0, 5));
    }

    #[test]
    fn dummy_provider_types() {
        let provider = DummyProvider;
        let types = provider.provide_types();
        assert_eq!(types.len(), 1);
        assert_eq!(types[0].base_type, Some("System.Object"));
        assert_eq!(types[0].fields, vec!["Value"]);
    }

    #[test]
    fn dummy_provider_methods() {
        let provider = DummyProvider;
        let methods = provider.provide_methods();
        assert_eq!(methods.len(), 1);
        assert!(methods[0].parameters.is_empty());
        assert_eq!(methods[0].return_type, "System.Int32");
    }

    // ── ProvidedType / ProvidedMethod clone + debug tests ────────────────

    #[test]
    fn provided_type_debug_and_clone() {
        let t = ProvidedType {
            full_name: "A.B",
            base_type: Some("C"),
            fields: vec!["x"],
        };
        let cloned = t.clone();
        assert_eq!(format!("{:?}", cloned), format!("{:?}", t));
    }

    #[test]
    fn provided_method_debug_and_clone() {
        let m = ProvidedMethod {
            type_name: "A.B",
            method_name: "Foo",
            parameters: vec!["System.Int32"],
            return_type: "System.Boolean",
        };
        let cloned = m.clone();
        assert_eq!(format!("{:?}", cloned), format!("{:?}", m));
    }
}
