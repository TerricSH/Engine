#[test]
fn install_local_parses_manifest_and_installs() {
    let (mut manager, tmp) = setup_manager();

    // Create manifest file.
    let manifest = sample_manifest();
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

    let pkg_dir = tmp.path().join("my_pkg");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("manifest.json"), &manifest_json).unwrap();
    std::fs::write(pkg_dir.join("data.bin"), b"test payload").unwrap();

    let result = manager.install_local(&pkg_dir.join("manifest.json"));
    assert!(result.is_ok(), "install_local failed: {result:?}");

    let pkg = result.unwrap();
    assert_eq!(pkg.state, PackageState::Active);
}

#[test]
fn default_manager_rejects_unsigned_manifest() {
    let tmp = tempfile::tempdir().unwrap();
    let mut manager =
        PackageManager::new(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5)).unwrap();
    let manifest = sample_manifest();
    let pkg_dir = tmp.path().join("unsigned-production");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let errors = manager
        .install_local(&pkg_dir.join("manifest.json"))
        .unwrap_err();
    assert!(errors
        .iter()
        .any(|error| matches!(error, UpdateError::SignatureMissing)));
    assert!(!tmp.path().join("download_temp").exists());
}

#[test]
fn production_manager_uses_configured_verifier_for_signed_package() {
    let tmp = tempfile::tempdir().unwrap();
    let rng = ring::rand::SystemRandom::new();
    let private_key = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
    let key_pair = ring::signature::Ed25519KeyPair::from_pkcs8(private_key.as_ref()).unwrap();

    let mut manifest = sample_manifest();
    crate::verify::sign_manifest_ed25519(
        &mut manifest,
        "release-2026",
        "2026-05-29T12:00:00Z",
        private_key.as_ref(),
    )
    .unwrap();
    let verifier = Verifier::production()
        .with_trusted_ed25519_key("release-2026", key_pair.public_key().as_ref())
        .unwrap();
    let mut manager = PackageManager::new_with_verifier(
        tmp.path(),
        PlatformKind::Desktop,
        "1.5.0",
        (1, 5),
        verifier,
    )
    .unwrap();

    let pkg_dir = tmp.path().join("signed-production");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(
        pkg_dir.join("manifest.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(pkg_dir.join("data.bin"), b"test payload").unwrap();

    let package = manager
        .install_local(&pkg_dir.join("manifest.json"))
        .unwrap();
    assert_eq!(package.state, PackageState::Active);
}

#[test]
fn install_local_rejects_incompatible_engine() {
    let (mut manager, tmp) = setup_manager();

    let mut manifest = sample_manifest();
    manifest.engine_version = "2.0.0".into();
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

    let pkg_dir = tmp.path().join("incompat");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("manifest.json"), &manifest_json).unwrap();

    let result = manager.install_local(&pkg_dir.join("manifest.json"));
    assert!(result.is_err());
}

#[test]
fn install_local_rejects_missing_payload() {
    let (mut manager, tmp) = setup_manager();

    let manifest = sample_manifest();
    let manifest_json = serde_json::to_string_pretty(&manifest).unwrap();

    let pkg_dir = tmp.path().join("missing_payload");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("manifest.json"), &manifest_json).unwrap();
    // data.bin NOT created — payload missing

    let result = manager.install_local(&pkg_dir.join("manifest.json"));
    assert!(result.is_err());
}

#[test]
fn install_local_invalid_json() {
    let (mut manager, tmp) = setup_manager();

    let pkg_dir = tmp.path().join("bad_json");
    std::fs::create_dir_all(&pkg_dir).unwrap();
    std::fs::write(pkg_dir.join("manifest.json"), "not valid json").unwrap();

    let result = manager.install_local(&pkg_dir.join("manifest.json"));
    assert!(result.is_err());
    assert!(matches!(
        result.unwrap_err().first().unwrap(),
        UpdateError::ManifestParse(_)
    ));
}

// ── rollback tests ─────────────────────────────────────────────────

#[test]
fn manager_rollback_restores_previous() {
    let (mut manager, tmp) = setup_manager();

    // Install first package.
    let m1 = sample_manifest();
    let j1 = serde_json::to_string_pretty(&m1).unwrap();
    let d1 = tmp.path().join("pkg1");
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::write(d1.join("manifest.json"), &j1).unwrap();
    std::fs::write(d1.join("data.bin"), b"test payload").unwrap();
    manager.install_local(&d1.join("manifest.json")).unwrap();
    let id1 = manager
        .active_package()
        .unwrap()
        .unwrap()
        .package_id()
        .to_string();
    manager.mark_boot_successful().unwrap();

    // Install second package.
    let mut m2 = sample_manifest();
    m2.created_at = "2026-06-01T00:00:00Z".into();
    let j2 = serde_json::to_string_pretty(&m2).unwrap();
    let d2 = tmp.path().join("pkg2");
    std::fs::create_dir_all(&d2).unwrap();
    std::fs::write(d2.join("manifest.json"), &j2).unwrap();
    std::fs::write(d2.join("data.bin"), b"test payload").unwrap();
    manager.install_local(&d2.join("manifest.json")).unwrap();

    // Rollback.
    let rolled = manager.rollback().unwrap();
    assert_eq!(rolled.state, PackageState::Active);

    // Active package should be the first one.
    let active = manager.active_package().unwrap().unwrap();
    assert_eq!(active.package_id(), id1);
}

#[test]
fn manager_rollback_fails_without_previous() {
    let (mut manager, _tmp) = setup_manager();
    let result = manager.rollback();
    assert!(result.is_err());
}

#[test]
fn unsafe_manifest_does_not_delete_existing_download_staging() {
    let tmp = tempfile::tempdir().unwrap();
    let cache_root = tmp.path().join("cache");
    let mut manager =
        PackageManager::new(&cache_root, PlatformKind::Desktop, "1.5.0", (1, 5)).unwrap();

    let download_dir = cache_root.join("download_temp");
    std::fs::create_dir_all(&download_dir).unwrap();
    let sentinel = download_dir.join("sentinel.txt");
    std::fs::write(&sentinel, b"keep").unwrap();

    let mut manifest = sample_manifest();
    manifest.payload_hashes[0].path = "../escape.bin".into();
    let package_dir = tmp.path().join("unsafe-package");
    std::fs::create_dir_all(&package_dir).unwrap();
    let manifest_path = package_dir.join("manifest.json");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let errors = manager.install_local(&manifest_path).unwrap_err();
    assert!(errors
        .iter()
        .any(|error| matches!(error, UpdateError::UnsafePath { .. })));
    assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
    assert!(!tmp.path().join("escape.bin").exists());
}

// ── check_boot tests ───────────────────────────────────────────────
