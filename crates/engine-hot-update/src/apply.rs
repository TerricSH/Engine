use std::path::Path;

use engine_asset::AssetRegistry;
use engine_serialize::{Diagnostic, DiagnosticSeverity, HotUpdateManifest, PlatformKind};
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// UpdateApplier
// ---------------------------------------------------------------------------

/// Runtime apply hooks for resource and logic asset updates.
///
/// After a package has been activated, these methods integrate the new
/// payloads with the running engine: reloading assets in the registry,
/// writing logic asset files, and (on Android) applying optional C#
/// assemblies.
pub struct UpdateApplier;

impl UpdateApplier {
    /// Apply resource updates through the asset registry.
    ///
    /// For every asset ID listed in the manifest's platform payloads that
    /// matches the current platform, the registry's `reload()` is called
    /// to refresh the cached asset from disk.
    ///
    /// Diagnostics are collected for each operation and returned.
    pub fn apply_resource_updates(
        manifest: &HotUpdateManifest,
        active_dir: &Path,
        registry: &mut AssetRegistry,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Collect all asset IDs from all platform payloads (the active
        // directory contains the correct payloads for the current platform).
        for payload in &manifest.platform_payloads {
            for asset_id in &payload.asset_ids {
                debug!(
                    asset = %asset_id.id,
                    "reloading asset from active package"
                );

                match registry.reload(asset_id) {
                    Ok(()) => {
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_RESOURCE_OK",
                                DiagnosticSeverity::Info,
                                "hot-update",
                                format!("asset reloaded: {}", asset_id.id),
                            )
                            .path(active_dir.display().to_string())
                            .contract("HotUpdate", "0.1"),
                        );
                    }
                    Err(e) => {
                        warn!(
                            asset = %asset_id.id,
                            error = %e,
                            "failed to reload asset"
                        );
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_RESOURCE_FAIL",
                                DiagnosticSeverity::Error,
                                "hot-update",
                                format!("failed to reload asset {}: {e}", asset_id.id),
                            )
                            .path(active_dir.display().to_string())
                            .contract("HotUpdate", "0.1"),
                        );
                    }
                }
            }
        }

        if diagnostics.is_empty() {
            info!("no resource updates to apply");
        }

        diagnostics
    }

    /// Apply interpreted logic asset updates.
    ///
    /// For each logic asset listed in the manifest, the payload file is
    /// expected to exist under `<active_dir>/<logic_asset_path>` and is
    /// copied to `assets/logic/<logic_asset_id>.<ext>` so the scripting
    /// runtime can pick it up.
    ///
    /// Currently the logic asset payload path is derived from the logic
    /// asset ID (mapped to a file name).  A future gate will use proper
    /// mapping metadata.
    pub fn apply_logic_assets(
        manifest: &HotUpdateManifest,
        active_dir: &Path,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        for payload in &manifest.platform_payloads {
            for logic_id in &payload.logic_asset_ids {
                // Derive the source path from the active dir.
                let source = active_dir.join(format!("logic/{logic_id}.lua"));

                let target_dir = Path::new("assets/logic");
                let target = target_dir.join(format!("{logic_id}.lua"));

                if !source.exists() {
                    warn!(
                        logic_id = %logic_id,
                        path = %source.display(),
                        "logic asset source not found"
                    );
                    diagnostics.push(
                        Diagnostic::new(
                            "HOT_UPDATE_LOGIC_MISSING",
                            DiagnosticSeverity::Warning,
                            "hot-update",
                            format!("logic asset source not found: {logic_id}"),
                        )
                        .path(source.display().to_string()),
                    );
                    continue;
                }

                // Ensure target directory exists.
                if let Err(e) = std::fs::create_dir_all(target_dir) {
                    warn!(
                        logic_id = %logic_id,
                        error = %e,
                        "cannot create logic asset directory"
                    );
                    diagnostics.push(
                        Diagnostic::new(
                            "HOT_UPDATE_LOGIC_DIR_FAIL",
                            DiagnosticSeverity::Error,
                            "hot-update",
                            format!("cannot create logic directory: {e}"),
                        )
                        .path(target_dir.display().to_string()),
                    );
                    continue;
                }

                match std::fs::copy(&source, &target) {
                    Ok(n) => {
                        debug!(
                            logic_id = %logic_id,
                            bytes = n,
                            "logic asset applied"
                        );
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_LOGIC_OK",
                                DiagnosticSeverity::Info,
                                "hot-update",
                                format!("logic asset applied: {logic_id}"),
                            )
                            .path(target.display().to_string()),
                        );
                    }
                    Err(e) => {
                        warn!(
                            logic_id = %logic_id,
                            error = %e,
                            "failed to copy logic asset"
                        );
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_LOGIC_COPY_FAIL",
                                DiagnosticSeverity::Error,
                                "hot-update",
                                format!("failed to copy logic asset {logic_id}: {e}"),
                            )
                            .path(target.display().to_string()),
                        );
                    }
                }
            }
        }

        diagnostics
    }

    /// Apply Android optional C# assembly payload.
    ///
    /// If the manifest contains an Android platform payload with an
    /// `optional_assembly`, the assembly file is copied from the active
    /// directory to `assets/assemblies/`.  On non-Android platforms this
    /// is a no-op.
    pub fn apply_android_assembly(
        manifest: &HotUpdateManifest,
        active_dir: &Path,
    ) -> Vec<Diagnostic> {
        let mut diagnostics = Vec::new();

        // Find Android payload with optional assembly.
        for payload in &manifest.platform_payloads {
            if payload.platform != PlatformKind::Android
                && payload.platform != PlatformKind::All
            {
                continue;
            }

            if let Some(ref assembly) = payload.optional_assembly {
                let source = active_dir.join(&assembly.path);
                if !source.exists() {
                    warn!(
                        path = %source.display(),
                        "Android assembly source not found"
                    );
                    diagnostics.push(
                        Diagnostic::new(
                            "HOT_UPDATE_ASSEMBLY_MISSING",
                            DiagnosticSeverity::Warning,
                            "hot-update",
                            format!("Android assembly not found: {}", assembly.path),
                        )
                        .path(source.display().to_string()),
                    );
                    continue;
                }

                let target_dir = Path::new("assets/assemblies");
                if let Err(e) = std::fs::create_dir_all(target_dir) {
                    diagnostics.push(
                        Diagnostic::new(
                            "HOT_UPDATE_ASSEMBLY_DIR_FAIL",
                            DiagnosticSeverity::Error,
                            "hot-update",
                            format!("cannot create assemblies directory: {e}"),
                        )
                        .path(target_dir.display().to_string()),
                    );
                    continue;
                }

                let target = target_dir.join(
                    Path::new(&assembly.path)
                        .file_name()
                        .unwrap_or_default(),
                );

                match std::fs::copy(&source, &target) {
                    Ok(n) => {
                        info!(
                            path = %assembly.path,
                            bytes = n,
                            "Android assembly applied"
                        );
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_ASSEMBLY_OK",
                                DiagnosticSeverity::Info,
                                "hot-update",
                                format!("Android assembly applied: {}", assembly.path),
                            )
                            .path(target.display().to_string()),
                        );
                    }
                    Err(e) => {
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_ASSEMBLY_COPY_FAIL",
                                DiagnosticSeverity::Error,
                                "hot-update",
                                format!("failed to copy assembly: {e}"),
                            )
                            .path(target.display().to_string()),
                        );
                    }
                }
            }
        }

        if diagnostics.is_empty() {
            diagnostics.push(
                Diagnostic::new(
                    "HOT_UPDATE_ASSEMBLY_NOOP",
                    DiagnosticSeverity::Info,
                    "hot-update",
                    "no Android assembly to apply",
                ),
            );
        }

        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssetId, AssemblyPayload, PlatformPayload, RollbackMetadata, SchemaVersion,
    };

    fn sample_manifest() -> HotUpdateManifest {
        HotUpdateManifest {
            manifest_version: SchemaVersion::new(0, 1, 0),
            engine_version: "1.5.0".into(),
            script_api_version: (1, 2),
            content_schema_version: SchemaVersion::new(1, 0, 0),
            logic_asset_schema_version: SchemaVersion::new(1, 0, 0),
            platform_payloads: vec![PlatformPayload {
                platform: PlatformKind::Desktop,
                asset_ids: vec![AssetId::new("mesh-cube")],
                logic_asset_ids: vec!["logic-player".into()],
                optional_assembly: None,
            }],
            payload_hashes: vec![],
            signature: None,
            rollback: RollbackMetadata {
                previous_manifest_hash: None,
                fallback_manifest_path: None,
                min_safe_engine_version: "1.4.0".into(),
            },
            created_at: "2026-05-29T12:00:00Z".into(),
        }
    }

    // ── Resource update tests ──────────────────────────────────────────

    #[test]
    fn apply_resource_updates_empty_manifest() {
        let manifest = sample_manifest();
        let mut registry = AssetRegistry::new();
        let dir = std::env::temp_dir().join("apply_res_empty");

        let diags = UpdateApplier::apply_resource_updates(&manifest, &dir, &mut registry);
        // Desktop payload has mesh-cube asset but it won't exist on disk,
        // so reload will fail.
        assert!(!diags.is_empty());
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_RESOURCE_FAIL"));
    }

    #[test]
    fn apply_resource_updates_no_payload_assets() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.clear();
        let mut registry = AssetRegistry::new();
        let dir = std::env::temp_dir().join("apply_res_none");

        let diags = UpdateApplier::apply_resource_updates(&manifest, &dir, &mut registry);
        assert!(diags.is_empty());
    }

    // ── Logic asset tests ──────────────────────────────────────────────

    #[test]
    fn apply_logic_assets_missing_source_produces_warning() {
        let manifest = sample_manifest();
        let dir = std::env::temp_dir().join("apply_logic_miss");

        let diags = UpdateApplier::apply_logic_assets(&manifest, &dir);
        assert!(!diags.is_empty());
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_LOGIC_MISSING"));
    }

    #[test]
    fn apply_logic_assets_copies_file() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::Desktop,
            asset_ids: vec![],
            logic_asset_ids: vec!["test-script".into()],
            optional_assembly: None,
        }];

        let dir = std::env::temp_dir().join("apply_logic_copy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir.join("logic")).unwrap();
        std::fs::write(&dir.join("logic/test-script.lua"), b"return 42").unwrap();

        let diags = UpdateApplier::apply_logic_assets(&manifest, &dir);

        // Should have at least an OK diagnostic.
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_LOGIC_OK"));

        // Clean up created file.
        let _ = std::fs::remove_file("assets/logic/test-script.lua");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_logic_assets_no_logic_ids() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::Desktop,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: None,
        }];

        let dir = std::env::temp_dir().join("apply_logic_empty");
        let diags = UpdateApplier::apply_logic_assets(&manifest, &dir);
        assert!(diags.is_empty());
    }

    // ── Android assembly tests ─────────────────────────────────────────

    #[test]
    fn apply_android_assembly_noop_on_non_android() {
        let manifest = sample_manifest(); // Desktop platform only
        let dir = std::env::temp_dir().join("apply_asm_noop");

        let diags = UpdateApplier::apply_android_assembly(&manifest, &dir);
        // Should have the NOOP diagnostic.
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_ASSEMBLY_NOOP"));
    }

    #[test]
    fn apply_android_assembly_copies_file() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(AssemblyPayload {
                path: "bin/GameAssembly.dll".into(),
                size_bytes: 100,
                hash: [0u8; 32],
                min_engine_version: "1.5.0".into(),
            }),
        }];

        let dir = std::env::temp_dir().join("apply_asm_copy");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir.join("bin")).unwrap();
        std::fs::write(&dir.join("bin/GameAssembly.dll"), b"assembly data").unwrap();

        let diags = UpdateApplier::apply_android_assembly(&manifest, &dir);
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_ASSEMBLY_OK"));

        // Clean up
        let _ = std::fs::remove_file("assets/assemblies/GameAssembly.dll");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn apply_android_assembly_missing_source_warns() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(AssemblyPayload {
                path: "bin/missing.dll".into(),
                size_bytes: 100,
                hash: [0u8; 32],
                min_engine_version: "1.5.0".into(),
            }),
        }];

        let dir = std::env::temp_dir().join("apply_asm_miss");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let diags = UpdateApplier::apply_android_assembly(&manifest, &dir);
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_ASSEMBLY_MISSING"));
    }

    #[test]
    fn apply_android_assembly_on_all_platform() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::All,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(AssemblyPayload {
                path: "all/asm.dll".into(),
                size_bytes: 100,
                hash: [0u8; 32],
                min_engine_version: "1.5.0".into(),
            }),
        }];

        let dir = std::env::temp_dir().join("apply_asm_all");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir.join("all")).unwrap();
        std::fs::write(&dir.join("all/asm.dll"), b"assembly").unwrap();

        let diags = UpdateApplier::apply_android_assembly(&manifest, &dir);
        // "All" platform is matched by the apply logic.
        let codes: Vec<_> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"HOT_UPDATE_ASSEMBLY_OK"),
            "expected ASSEMBLY_OK, got: {codes:?}"
        );

        let _ = std::fs::remove_file("assets/assemblies/asm.dll");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
