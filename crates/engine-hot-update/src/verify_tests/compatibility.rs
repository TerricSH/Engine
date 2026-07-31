#[test]
fn verify_compatibility_accepts() {
    let manifest = sample_manifest();
    assert!(Verifier::verify_compatibility(&manifest, "1.5.0", (1, 5)).is_ok());
}

#[test]
fn verify_compatibility_rejects_engine_mismatch() {
    let manifest = sample_manifest();
    let result = Verifier::verify_compatibility(&manifest, "2.0.0", (1, 5));
    assert!(result.is_err());
    assert!(matches!(result, Err(UpdateError::IncompatibleVersion(_))));
}

#[test]
fn verify_compatibility_rejects_script_api() {
    let mut manifest = sample_manifest();
    manifest.script_api_version = (5, 0);
    let result = Verifier::verify_compatibility(&manifest, "1.5.0", (1, 5));
    assert!(result.is_err());
}

// ── Platform rule tests ─────────────────────────────────────────────

#[test]
fn verify_platform_rules_desktop_accepted() {
    let manifest = sample_manifest();
    assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::Desktop).is_ok());
}

#[test]
fn verify_platform_rules_ios_rejects_assemblies() {
    let mut manifest = sample_manifest();
    manifest.platform_payloads.push(PlatformPayload {
        platform: PlatformKind::Ios,
        asset_ids: vec![],
        logic_asset_ids: vec![],
        optional_assembly: Some(engine_serialize::AssemblyPayload {
            path: "ios/asm.dll".into(),
            size_bytes: 100,
            hash: [0xBB; 32],
            min_engine_version: "1.5.0".into(),
        }),
    });
    let result = Verifier::verify_platform_rules(&manifest, &PlatformKind::Ios);
    assert!(result.is_err());
    assert!(matches!(result, Err(UpdateError::PlatformRejected(_))));
}

#[test]
fn verify_platform_rules_android_allows_assemblies() {
    let mut manifest = sample_manifest();
    manifest.platform_payloads.push(PlatformPayload {
        platform: PlatformKind::Android,
        asset_ids: vec![],
        logic_asset_ids: vec![],
        optional_assembly: Some(engine_serialize::AssemblyPayload {
            path: "android/asm.dll".into(),
            size_bytes: 100,
            hash: [0xCC; 32],
            min_engine_version: "1.5.0".into(),
        }),
    });
    manifest.payload_hashes.push(PayloadHash {
        platform: PlatformKind::Android,
        path: "android/asm.dll".into(),
        algorithm: "sha256".into(),
        hash: [0xCC; 32],
    });
    assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).is_ok());
}

#[test]
fn verify_platform_rules_rejects_selected_assembly_without_selected_hash() {
    let mut manifest = sample_manifest();
    manifest.platform_payloads.push(PlatformPayload {
        platform: PlatformKind::Android,
        asset_ids: vec![],
        logic_asset_ids: vec![],
        optional_assembly: Some(engine_serialize::AssemblyPayload {
            path: "android/unhashed.dll".into(),
            size_bytes: 100,
            hash: [0xDD; 32],
            min_engine_version: "1.5.0".into(),
        }),
    });

    let error = Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).unwrap_err();
    assert!(
        matches!(error, UpdateError::PlatformRejected(message) if message.contains("no selected payload hash"))
    );
}

#[test]
fn verify_platform_rules_rejects_assembly_hash_disagreement() {
    let mut manifest = sample_manifest();
    manifest.platform_payloads.push(PlatformPayload {
        platform: PlatformKind::Android,
        asset_ids: vec![],
        logic_asset_ids: vec![],
        optional_assembly: Some(engine_serialize::AssemblyPayload {
            path: "android/mismatch.dll".into(),
            size_bytes: 100,
            hash: [0xDD; 32],
            min_engine_version: "1.5.0".into(),
        }),
    });
    manifest.payload_hashes.push(PayloadHash {
        platform: PlatformKind::Android,
        path: "android/mismatch.dll".into(),
        algorithm: "sha256".into(),
        hash: [0xEE; 32],
    });

    let error = Verifier::verify_platform_rules(&manifest, &PlatformKind::Android).unwrap_err();
    assert!(
        matches!(error, UpdateError::PlatformRejected(message) if message.contains("disagrees"))
    );
}

#[test]
fn verify_platform_rules_all_platform_no_assembly() {
    let manifest = sample_manifest();
    assert!(Verifier::verify_platform_rules(&manifest, &PlatformKind::All).is_ok());
}

// ── Cooked header tests ─────────────────────────────────────────────
