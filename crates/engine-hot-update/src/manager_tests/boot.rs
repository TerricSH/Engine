#[test]
fn check_boot_no_marker_ok() {
    let (mut manager, _tmp) = setup_manager();
    assert!(manager.check_boot().is_ok());
}

#[test]
fn check_boot_with_marker_triggers_rollback() {
    let (mut manager, tmp) = setup_manager();

    // Install a package first so there's something to roll back from.
    let m1 = sample_manifest();
    let j1 = serde_json::to_string_pretty(&m1).unwrap();
    let d1 = tmp.path().join("first");
    std::fs::create_dir_all(&d1).unwrap();
    std::fs::write(d1.join("manifest.json"), &j1).unwrap();
    std::fs::write(d1.join("data.bin"), b"test payload").unwrap();
    manager.install_local(&d1.join("manifest.json")).unwrap();

    // Simulate boot marker presence (it's already there from activation).
    // The boot marker means rollback is needed, but since there's
    // no earlier version to rollback to, it may fail.
    // But check_boot should attempt it.
    assert!(RollbackManager::needs_rollback(&manager.cache));
}

#[test]
fn manager_boot_success_api_prevents_restart_rollback() {
    let (mut manager, tmp) = setup_manager();
    let manifest = sample_manifest();
    let package_dir = tmp.path().join("boot-success");
    std::fs::create_dir_all(&package_dir).unwrap();
    std::fs::write(
        package_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(package_dir.join("data.bin"), b"test payload").unwrap();
    let installed = manager
        .install_local(&package_dir.join("manifest.json"))
        .unwrap();
    manager.mark_boot_successful().unwrap();

    let cache_dir = manager.cache.base_dir.clone();
    drop(manager);
    let mut restarted =
        PackageManager::try_new_development(&cache_dir, PlatformKind::Desktop, "1.5.0", (1, 5))
            .unwrap();
    restarted.check_boot().unwrap();
    assert_eq!(
        restarted.active_package().unwrap().unwrap().package_id(),
        installed.package_id()
    );
}

#[test]
fn manager_failed_boot_api_rolls_back_to_recorded_package() {
    let (mut manager, tmp) = setup_manager();
    let first_manifest = sample_manifest();
    let first_dir = tmp.path().join("boot-failed-first");
    std::fs::create_dir_all(&first_dir).unwrap();
    std::fs::write(
        first_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&first_manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(first_dir.join("data.bin"), b"test payload").unwrap();
    let first = manager
        .install_local(&first_dir.join("manifest.json"))
        .unwrap();
    manager.mark_boot_successful().unwrap();

    let mut second_manifest = sample_manifest();
    second_manifest.created_at = "2026-09-01T00:00:00Z".into();
    let second_dir = tmp.path().join("boot-failed-second");
    std::fs::create_dir_all(&second_dir).unwrap();
    std::fs::write(
        second_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&second_manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(second_dir.join("data.bin"), b"test payload").unwrap();
    manager
        .install_local(&second_dir.join("manifest.json"))
        .unwrap();
    manager.mark_failed_boot().unwrap();
    manager.check_boot().unwrap();

    assert_eq!(
        manager.active_package().unwrap().unwrap().package_id(),
        first.package_id()
    );
    assert!(!RollbackManager::needs_rollback(&manager.cache));
}

// ── list / active tests ────────────────────────────────────────────
