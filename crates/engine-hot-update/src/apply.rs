use std::collections::BTreeSet;
use std::path::Path;

use engine_asset::{asset_relative_path, AssetRegistry};
use engine_serialize::{Diagnostic, DiagnosticSeverity, HotUpdateManifest, PlatformKind};
use tracing::{debug, info, warn};

use crate::error::UpdateError;
use crate::path_safety::{safe_join, validate_manifest_paths};

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
    /// For every selected asset ID, resolve `<active_dir>/assets/...`, require
    /// that exact relative path to be covered by a selected signed payload
    /// hash, and transactionally replace the registry cache from that root.
    ///
    /// Diagnostics are collected for each operation and returned.
    pub fn apply_resource_updates(
        manifest: &HotUpdateManifest,
        active_dir: &Path,
        registry: &mut AssetRegistry,
        platform: &PlatformKind,
    ) -> Vec<Diagnostic> {
        if let Err(errors) = validate_manifest_paths(manifest) {
            return path_error_diagnostics(errors);
        }

        let mut diagnostics = Vec::new();
        let selected_hash_paths: BTreeSet<&str> = manifest
            .payload_hashes_for_platform(*platform)
            .into_iter()
            .map(|payload| payload.path.as_str())
            .collect();

        for payload in manifest.payloads_for_platform(*platform) {
            for asset_id in &payload.asset_ids {
                let relative_path = match asset_relative_path(asset_id) {
                    Ok(path) => path,
                    Err(error) => {
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_PATH_REJECTED",
                                DiagnosticSeverity::Error,
                                "hot-update",
                                error.to_string(),
                            )
                            .contract("HotUpdate", "0.1"),
                        );
                        continue;
                    }
                };
                let source_path = active_dir.join(&relative_path);
                if !selected_hash_paths.contains(relative_path.as_str()) {
                    warn!(
                        asset = %asset_id.id,
                        path = %relative_path,
                        "asset is not covered by a selected payload hash"
                    );
                    diagnostics.push(
                        Diagnostic::new(
                            "HOT_UPDATE_RESOURCE_UNMAPPED",
                            DiagnosticSeverity::Error,
                            "hot-update",
                            format!(
                                "asset {} is not mapped to selected signed payload {relative_path}",
                                asset_id.id
                            ),
                        )
                        .path(source_path.display().to_string())
                        .contract("HotUpdate", "0.1"),
                    );
                    continue;
                }
                debug!(
                    asset = %asset_id.id,
                    path = %source_path.display(),
                    "reloading asset from active package"
                );

                match registry.reload_from_root(asset_id, active_dir) {
                    Ok(()) => {
                        diagnostics.push(
                            Diagnostic::new(
                                "HOT_UPDATE_RESOURCE_OK",
                                DiagnosticSeverity::Info,
                                "hot-update",
                                format!("asset reloaded: {}", asset_id.id),
                            )
                            .path(source_path.display().to_string())
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
                            .path(source_path.display().to_string())
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
        platform: &PlatformKind,
    ) -> Vec<Diagnostic> {
        if let Err(errors) = validate_manifest_paths(manifest) {
            return path_error_diagnostics(errors);
        }

        let target_dir = Path::new("assets/logic");
        let mut prepared = Vec::new();
        for payload in manifest.payloads_for_platform(*platform) {
            for logic_id in &payload.logic_asset_ids {
                let source = match safe_join(
                    active_dir,
                    &format!("logic/{logic_id}.lua"),
                    "logic asset source",
                ) {
                    Ok(path) => path,
                    Err(error) => return path_error_diagnostics(vec![error]),
                };
                let target = match safe_join(
                    target_dir,
                    &format!("{logic_id}.lua"),
                    "logic asset destination",
                ) {
                    Ok(path) => path,
                    Err(error) => return path_error_diagnostics(vec![error]),
                };
                prepared.push((logic_id, source, target));
            }
        }

        let mut diagnostics = Vec::new();
        for (logic_id, source, target) in prepared {
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
        platform: &PlatformKind,
    ) -> Vec<Diagnostic> {
        if let Err(errors) = validate_manifest_paths(manifest) {
            return path_error_diagnostics(errors);
        }

        let target_dir = Path::new("assets/assemblies");
        let mut prepared = Vec::new();
        if *platform == PlatformKind::Android {
            for payload in manifest.payloads_for_platform(*platform) {
                let Some(assembly) = &payload.optional_assembly else {
                    continue;
                };
                let source = match safe_join(active_dir, &assembly.path, "assembly source") {
                    Ok(path) => path,
                    Err(error) => return path_error_diagnostics(vec![error]),
                };
                let Some(file_name) = Path::new(&assembly.path)
                    .file_name()
                    .and_then(|v| v.to_str())
                else {
                    return path_error_diagnostics(vec![UpdateError::UnsafePath {
                        field: "assembly destination".into(),
                        path: assembly.path.clone(),
                        reason: "assembly path has no valid file name".into(),
                    }]);
                };
                let target = match safe_join(target_dir, file_name, "assembly destination") {
                    Ok(path) => path,
                    Err(error) => return path_error_diagnostics(vec![error]),
                };
                prepared.push((assembly, source, target));
            }
        }

        let mut diagnostics = Vec::new();
        for (assembly, source, target) in prepared {
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

        if diagnostics.is_empty() {
            diagnostics.push(Diagnostic::new(
                "HOT_UPDATE_ASSEMBLY_NOOP",
                DiagnosticSeverity::Info,
                "hot-update",
                "no Android assembly to apply",
            ));
        }

        diagnostics
    }
}

fn path_error_diagnostics(errors: Vec<UpdateError>) -> Vec<Diagnostic> {
    errors
        .into_iter()
        .map(|error| {
            Diagnostic::new(
                "HOT_UPDATE_PATH_REJECTED",
                DiagnosticSeverity::Error,
                "hot-update",
                error.to_string(),
            )
            .contract("HotUpdate", "0.1")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine_serialize::{
        AssemblyPayload, AssetId, PayloadHash, PlatformPayload, RollbackMetadata, SchemaVersion,
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

        let diags = UpdateApplier::apply_resource_updates(
            &manifest,
            &dir,
            &mut registry,
            &PlatformKind::Desktop,
        );
        // The asset is not mapped to a selected signed payload hash.
        assert!(!diags.is_empty());
        assert!(diags
            .iter()
            .any(|d| d.code == "HOT_UPDATE_RESOURCE_UNMAPPED"));
    }

    #[test]
    fn apply_resource_updates_no_payload_assets() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads.clear();
        let mut registry = AssetRegistry::new();
        let dir = std::env::temp_dir().join("apply_res_none");

        let diags = UpdateApplier::apply_resource_updates(
            &manifest,
            &dir,
            &mut registry,
            &PlatformKind::Desktop,
        );
        assert!(diags.is_empty());
    }

    fn resource_manifest(asset_id: AssetId) -> HotUpdateManifest {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::Desktop,
            asset_ids: vec![asset_id],
            logic_asset_ids: Vec::new(),
            optional_assembly: None,
        }];
        manifest.payload_hashes = vec![PayloadHash {
            platform: PlatformKind::Desktop,
            path: "assets/textures/runtime.bin".into(),
            algorithm: "sha256".into(),
            hash: [7; 32],
        }];
        manifest
    }

    fn seed_cached_asset(registry: &mut AssetRegistry, root: &Path, asset_id: &AssetId) {
        let source = root.join("assets/textures/runtime.bin");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(source, b"old bytes").unwrap();
        registry.reload_from_root(asset_id, root).unwrap();
        assert_eq!(
            registry.load(asset_id).unwrap().get().as_slice(),
            b"old bytes"
        );
    }

    #[test]
    fn apply_resource_updates_reads_new_bytes_from_active_package() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let active = temp.path().join("active");
        let asset_id = AssetId::with_path("texture-runtime", "textures/runtime.bin");
        let manifest = resource_manifest(asset_id.clone());
        let mut registry = AssetRegistry::new();
        seed_cached_asset(&mut registry, &base, &asset_id);
        let source = active.join("assets/textures/runtime.bin");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(&source, b"new bytes").unwrap();

        let diagnostics = UpdateApplier::apply_resource_updates(
            &manifest,
            &active,
            &mut registry,
            &PlatformKind::Desktop,
        );

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "HOT_UPDATE_RESOURCE_OK"));
        assert_eq!(
            registry.load(&asset_id).unwrap().get().as_slice(),
            b"new bytes"
        );
    }

    #[test]
    fn missing_active_resource_keeps_old_cached_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let active = temp.path().join("active");
        let asset_id = AssetId::with_path("texture-runtime", "textures/runtime.bin");
        let manifest = resource_manifest(asset_id.clone());
        let mut registry = AssetRegistry::new();
        seed_cached_asset(&mut registry, &base, &asset_id);

        let diagnostics = UpdateApplier::apply_resource_updates(
            &manifest,
            &active,
            &mut registry,
            &PlatformKind::Desktop,
        );

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "HOT_UPDATE_RESOURCE_FAIL"));
        assert_eq!(
            registry.load(&asset_id).unwrap().get().as_slice(),
            b"old bytes"
        );
    }

    #[test]
    fn unsigned_extra_resource_is_not_loaded_and_keeps_old_cache() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let active = temp.path().join("active");
        let asset_id = AssetId::with_path("texture-runtime", "textures/runtime.bin");
        let mut manifest = resource_manifest(asset_id.clone());
        manifest.payload_hashes.clear();
        let mut registry = AssetRegistry::new();
        seed_cached_asset(&mut registry, &base, &asset_id);
        let source = active.join("assets/textures/runtime.bin");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(source, b"unsigned bytes").unwrap();

        let diagnostics = UpdateApplier::apply_resource_updates(
            &manifest,
            &active,
            &mut registry,
            &PlatformKind::Desktop,
        );

        assert!(diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "HOT_UPDATE_RESOURCE_UNMAPPED"));
        assert_eq!(
            registry.load(&asset_id).unwrap().get().as_slice(),
            b"old bytes"
        );
    }

    #[test]
    fn malicious_asset_paths_reject_entire_apply_before_cache_mutation() {
        let temp = tempfile::tempdir().unwrap();
        let base = temp.path().join("base");
        let active = temp.path().join("active");
        let safe_id = AssetId::with_path("texture-runtime", "textures/runtime.bin");
        let mut manifest = resource_manifest(safe_id.clone());
        manifest.platform_payloads[0].asset_ids.extend([
            AssetId::new("../escape"),
            AssetId::with_path("texture-evil", "..\\escape.bin"),
        ]);
        let mut registry = AssetRegistry::new();
        seed_cached_asset(&mut registry, &base, &safe_id);
        let source = active.join("assets/textures/runtime.bin");
        std::fs::create_dir_all(source.parent().unwrap()).unwrap();
        std::fs::write(source, b"new bytes").unwrap();

        let diagnostics = UpdateApplier::apply_resource_updates(
            &manifest,
            &active,
            &mut registry,
            &PlatformKind::Desktop,
        );

        assert!(diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code == "HOT_UPDATE_PATH_REJECTED"));
        assert_eq!(
            registry.load(&safe_id).unwrap().get().as_slice(),
            b"old bytes"
        );
    }

    // ── Logic asset tests ──────────────────────────────────────────────

    #[test]
    fn apply_logic_assets_missing_source_produces_warning() {
        let manifest = sample_manifest();
        let dir = std::env::temp_dir().join("apply_logic_miss");

        let diags = UpdateApplier::apply_logic_assets(&manifest, &dir, &PlatformKind::Desktop);
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
        std::fs::create_dir_all(dir.join("logic")).unwrap();
        std::fs::write(dir.join("logic/test-script.lua"), b"return 42").unwrap();

        let diags = UpdateApplier::apply_logic_assets(&manifest, &dir, &PlatformKind::Desktop);

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
        let diags = UpdateApplier::apply_logic_assets(&manifest, &dir, &PlatformKind::Desktop);
        assert!(diags.is_empty());
    }

    #[test]
    fn apply_logic_assets_only_applies_current_platform_and_all() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![
            PlatformPayload {
                platform: PlatformKind::Desktop,
                asset_ids: vec![],
                logic_asset_ids: vec!["platform-desktop".into()],
                optional_assembly: None,
            },
            PlatformPayload {
                platform: PlatformKind::Android,
                asset_ids: vec![],
                logic_asset_ids: vec!["platform-android-missing".into()],
                optional_assembly: None,
            },
            PlatformPayload {
                platform: PlatformKind::All,
                asset_ids: vec![],
                logic_asset_ids: vec!["platform-common".into()],
                optional_assembly: None,
            },
        ];
        let temp = tempfile::tempdir().unwrap();
        let logic = temp.path().join("logic");
        std::fs::create_dir_all(&logic).unwrap();
        std::fs::write(logic.join("platform-desktop.lua"), b"desktop").unwrap();
        std::fs::write(logic.join("platform-common.lua"), b"common").unwrap();

        let diagnostics =
            UpdateApplier::apply_logic_assets(&manifest, temp.path(), &PlatformKind::Desktop);

        assert_eq!(
            diagnostics
                .iter()
                .filter(|diagnostic| diagnostic.code == "HOT_UPDATE_LOGIC_OK")
                .count(),
            2
        );
        assert!(!diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("platform-android-missing")));
        let _ = std::fs::remove_file("assets/logic/platform-desktop.lua");
        let _ = std::fs::remove_file("assets/logic/platform-common.lua");
    }

    #[test]
    fn apply_logic_assets_rejects_malicious_id_before_copying() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads[0].logic_asset_ids = vec!["../escaped".into()];
        let temp = tempfile::tempdir().unwrap();

        let diagnostics =
            UpdateApplier::apply_logic_assets(&manifest, temp.path(), &PlatformKind::Desktop);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "HOT_UPDATE_PATH_REJECTED");
        assert_eq!(diagnostics[0].severity, DiagnosticSeverity::Error);
    }

    // ── Android assembly tests ─────────────────────────────────────────

    #[test]
    fn apply_android_assembly_noop_on_non_android() {
        let manifest = sample_manifest(); // Desktop platform only
        let dir = std::env::temp_dir().join("apply_asm_noop");

        let diags = UpdateApplier::apply_android_assembly(&manifest, &dir, &PlatformKind::Desktop);
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

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/GameAssembly.dll"), b"assembly data").unwrap();

        let diags = UpdateApplier::apply_android_assembly(&manifest, dir, &PlatformKind::Android);
        assert!(diags.iter().any(|d| d.code == "HOT_UPDATE_ASSEMBLY_OK"));

        // Clean up
        let _ = std::fs::remove_file("assets/assemblies/GameAssembly.dll");
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

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();

        let diags = UpdateApplier::apply_android_assembly(&manifest, dir, &PlatformKind::Android);
        assert!(diags
            .iter()
            .any(|d| d.code == "HOT_UPDATE_ASSEMBLY_MISSING"));
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

        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path();
        std::fs::create_dir_all(dir.join("all")).unwrap();
        std::fs::write(dir.join("all/asm.dll"), b"assembly").unwrap();

        let diags = UpdateApplier::apply_android_assembly(&manifest, dir, &PlatformKind::Android);
        // "All" platform is matched by the apply logic.
        let codes: Vec<_> = diags.iter().map(|d| d.code.as_str()).collect();
        assert!(
            codes.contains(&"HOT_UPDATE_ASSEMBLY_OK"),
            "expected ASSEMBLY_OK, got: {codes:?}"
        );

        let _ = std::fs::remove_file("assets/assemblies/asm.dll");
    }

    #[test]
    fn apply_android_assembly_rejects_traversal_before_copying() {
        let mut manifest = sample_manifest();
        manifest.platform_payloads = vec![PlatformPayload {
            platform: PlatformKind::Android,
            asset_ids: vec![],
            logic_asset_ids: vec![],
            optional_assembly: Some(AssemblyPayload {
                path: "../../outside.dll".into(),
                size_bytes: 1,
                hash: [0u8; 32],
                min_engine_version: "1.5.0".into(),
            }),
        }];
        let temp = tempfile::tempdir().unwrap();

        let diagnostics =
            UpdateApplier::apply_android_assembly(&manifest, temp.path(), &PlatformKind::Android);
        assert_eq!(diagnostics.len(), 1);
        assert_eq!(diagnostics[0].code, "HOT_UPDATE_PATH_REJECTED");
    }
}
