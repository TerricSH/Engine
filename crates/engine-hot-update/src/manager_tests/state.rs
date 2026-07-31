#[test]
fn list_packages_after_install() {
    let (mut manager, tmp) = setup_manager();

    let manifest = sample_manifest();
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    let d = tmp.path().join("list_test");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), &json).unwrap();
    std::fs::write(d.join("data.bin"), b"test payload").unwrap();
    manager.install_local(&d.join("manifest.json")).unwrap();

    let packages = manager.list_packages().unwrap();
    assert!(!packages.is_empty());
}

#[test]
fn active_package_returns_some_after_install() {
    let (mut manager, tmp) = setup_manager();

    let manifest = sample_manifest();
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    let d = tmp.path().join("active_test");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), &json).unwrap();
    std::fs::write(d.join("data.bin"), b"test payload").unwrap();
    manager.install_local(&d.join("manifest.json")).unwrap();

    assert!(manager.active_package().unwrap().is_some());
}

#[test]
fn active_package_none_before_install() {
    let (manager, _tmp) = setup_manager();
    assert!(manager.active_package().unwrap().is_none());
}

// ── apply_updates tests ────────────────────────────────────────────

#[test]
fn apply_updates_no_active_package() {
    let (mut manager, _tmp) = setup_manager();
    let mut registry = AssetRegistry::new();
    let diags = manager.apply_updates(&mut registry);
    assert!(diags.is_empty());
}

#[test]
fn apply_updates_after_install() {
    let (mut manager, tmp) = setup_manager();

    let manifest = sample_manifest();
    let json = serde_json::to_string_pretty(&manifest).unwrap();
    let d = tmp.path().join("apply_test");
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("manifest.json"), &json).unwrap();
    std::fs::write(d.join("data.bin"), b"test payload").unwrap();
    manager.install_local(&d.join("manifest.json")).unwrap();

    let mut registry = AssetRegistry::new();
    let diags = manager.apply_updates(&mut registry);

    // Should have at least some diagnostics (logic asset missing, etc.)
    assert!(!diags.is_empty());
}

// ── Edge-case tests ────────────────────────────────────────────────

#[test]
fn manager_new_initializes_cache() {
    let tmp = tempfile::tempdir().unwrap();
    let _manager = PackageManager::new(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5)).unwrap();

    // Cache directories should exist.
    assert!(tmp.path().join("packages").exists());
    assert!(tmp.path().join("staged").exists());
    assert!(tmp.path().join("active").exists());
}

#[test]
fn strict_constructor_fails_when_cache_cannot_be_initialized() {
    let tmp = tempfile::tempdir().unwrap();
    let invalid_root = tmp.path().join("not-a-directory");
    std::fs::write(&invalid_root, b"file").unwrap();

    assert!(PackageManager::try_new_development(
        &invalid_root,
        PlatformKind::Desktop,
        "1.5.0",
        (1, 5),
    )
    .is_err());
}

#[test]
fn cache_lock_rejects_second_manager_until_owner_drops() {
    let tmp = tempfile::tempdir().unwrap();
    let first =
        PackageManager::try_new_development(tmp.path(), PlatformKind::Desktop, "1.5.0", (1, 5))
            .unwrap();

    let error = match PackageManager::try_new_development(
        tmp.path(),
        PlatformKind::Desktop,
        "1.5.0",
        (1, 5),
    ) {
        Ok(_) => panic!("second manager unexpectedly acquired the cache lock"),
        Err(error) => error,
    };
    assert!(error.to_string().contains("already owned"));

    drop(first);
    assert!(PackageManager::try_new_development(
        tmp.path(),
        PlatformKind::Desktop,
        "1.5.0",
        (1, 5),
    )
    .is_ok());
}

#[test]
fn apply_updates_fails_closed_when_transaction_recovery_fails() {
    let (mut manager, _tmp) = setup_manager();
    manager
        .cache
        .write_transaction(&crate::transaction::ActivationTransaction {
            version: crate::transaction::ACTIVATION_FORMAT_VERSION,
            operation: crate::transaction::TransactionOperation::Activate,
            activated_id: "a".repeat(64),
            previous_id: Some("b".repeat(64)),
            moved_staged_to_active: false,
        })
        .unwrap();

    let mut registry = AssetRegistry::new();
    let diagnostics = manager.apply_updates(&mut registry);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, "HOT_UPDATE_RECOVERY_FAILED");
    assert!(registry.cached_ids().is_empty());
    assert!(manager.try_active_package().is_err());
    assert!(manager.active_package().is_err());
}

#[test]
fn install_local_multiple_times() {
    let (mut manager, tmp) = setup_manager();

    for i in 0..3 {
        let mut m = sample_manifest();
        m.created_at = format!("2026-06-{:02}T00:00:00Z", i + 1);
        let json = serde_json::to_string_pretty(&m).unwrap();
        let d = tmp.path().join(format!("multi_{i}"));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("manifest.json"), &json).unwrap();
        std::fs::write(d.join("data.bin"), b"test payload").unwrap();
        let result = manager.install_local(&d.join("manifest.json"));
        assert!(result.is_ok(), "install {i} failed: {result:?}");
        manager.mark_boot_successful().unwrap();
    }

    // The last installed package should be active.
    let active = manager.active_package().unwrap().unwrap();
    assert_eq!(active.manifest.created_at, "2026-06-03T00:00:00Z");
}
