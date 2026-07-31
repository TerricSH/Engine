fn stage_test_package(
    cache: &PackageCache,
    root: &Path,
    manifest: &HotUpdateManifest,
    name: &str,
) -> Package {
    let download = root.join(name);
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(download.join("data.bin"), b"test payload").unwrap();
    Installer::stage(manifest, &download, cache, &PlatformKind::Desktop).unwrap()
}

fn install_confirmed_base(cache: &PackageCache, root: &Path) -> Package {
    let manifest = sample_manifest();
    let mut package = stage_test_package(cache, root, &manifest, "confirmed-base");
    Installer::activate(&mut package, cache, &PlatformKind::Desktop).unwrap();
    Installer::mark_boot_successful(cache).unwrap();
    package
}

#[test]
fn every_precommit_failure_restores_old_pointer_payload_state_and_markers() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(temp.path());
    cache.initialize().unwrap();
    let old = install_confirmed_base(&cache, temp.path());

    let mut manifest = sample_manifest();
    manifest.created_at = "2026-08-01T00:00:00Z".into();
    let mut next = stage_test_package(&cache, temp.path(), &manifest, "failure-next");
    let fail_points = [
        ActivationFailPoint::AfterJournal,
        ActivationFailPoint::AfterPayloadPrepared,
        ActivationFailPoint::AfterStatePrepared,
        ActivationFailPoint::AfterBootMarker,
        ActivationFailPoint::PointerReplaceFailure,
    ];

    for fail_point in fail_points {
        let error =
            Installer::activate_inner(&mut next, &cache, &PlatformKind::Desktop, Some(fail_point))
                .unwrap_err();
        assert!(matches!(error, UpdateError::ActivationFailed(_)));
        assert_eq!(
            cache.active_package_id().unwrap().as_deref(),
            Some(old.package_id())
        );
        assert!(cache.active_dir(old.package_id()).unwrap().is_dir());
        assert!(!cache.active_dir(next.package_id()).unwrap().exists());
        assert!(cache.staged_dir(next.package_id()).unwrap().is_dir());
        assert_eq!(
            cache.read_state(next.package_id()).unwrap().state,
            PackageState::Staged
        );
        assert_eq!(next.state, PackageState::Staged);
        assert!(!cache.boot_marker_path().exists());
        assert!(!cache.transaction_path().unwrap().exists());
        let record = cache.read_activation_record().unwrap().unwrap();
        assert_eq!(record.activated_id, old.package_id());
        assert_eq!(record.phase, ActivationPhase::BootSuccessful);
    }
}

#[test]
fn startup_recovers_crash_before_pointer_commit() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(temp.path());
    cache.initialize().unwrap();
    let old = install_confirmed_base(&cache, temp.path());

    let mut manifest = sample_manifest();
    manifest.created_at = "2026-08-02T00:00:00Z".into();
    let mut next = stage_test_package(&cache, temp.path(), &manifest, "crash-before-next");
    let transaction = ActivationTransaction {
        version: ACTIVATION_FORMAT_VERSION,
        operation: TransactionOperation::Activate,
        activated_id: next.package_id().to_string(),
        previous_id: Some(old.package_id().to_string()),
        moved_staged_to_active: true,
    };
    cache.write_transaction(&transaction).unwrap();
    std::fs::rename(next.staging_dir(), next.active_dir()).unwrap();
    next.state = PackageState::Active;
    cache.write_state(&next).unwrap();
    cache
        .write_boot_marker(&ActivationRecord::new(
            next.package_id().to_string(),
            Some(old.package_id().to_string()),
            ActivationPhase::BootPending,
        ))
        .unwrap();

    let restarted = PackageCache::new(temp.path());
    restarted.initialize().unwrap();
    assert_eq!(
        restarted.active_package_id().unwrap().as_deref(),
        Some(old.package_id())
    );
    assert!(restarted.staged_dir(next.package_id()).unwrap().is_dir());
    assert!(!restarted.active_dir(next.package_id()).unwrap().exists());
    assert_eq!(
        restarted.read_state(next.package_id()).unwrap().state,
        PackageState::Staged
    );
    assert!(!restarted.transaction_path().unwrap().exists());
    assert!(!restarted.boot_marker_path().exists());
}

#[test]
fn startup_finishes_crash_after_pointer_commit_without_rollback() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(temp.path());
    cache.initialize().unwrap();
    let old = install_confirmed_base(&cache, temp.path());

    let mut manifest = sample_manifest();
    manifest.created_at = "2026-08-03T00:00:00Z".into();
    let mut next = stage_test_package(&cache, temp.path(), &manifest, "crash-after-next");
    let transaction = ActivationTransaction {
        version: ACTIVATION_FORMAT_VERSION,
        operation: TransactionOperation::Activate,
        activated_id: next.package_id().to_string(),
        previous_id: Some(old.package_id().to_string()),
        moved_staged_to_active: true,
    };
    cache.write_transaction(&transaction).unwrap();
    std::fs::rename(next.staging_dir(), next.active_dir()).unwrap();
    next.state = PackageState::Active;
    cache.write_state(&next).unwrap();
    cache.set_active_pointer(next.package_id()).unwrap();

    let restarted = PackageCache::new(temp.path());
    restarted.initialize().unwrap();
    assert_eq!(
        restarted.active_package_id().unwrap().as_deref(),
        Some(next.package_id())
    );
    assert!(restarted.active_dir(old.package_id()).unwrap().is_dir());
    assert!(restarted.active_dir(next.package_id()).unwrap().is_dir());
    assert!(!restarted.transaction_path().unwrap().exists());
    assert!(restarted.boot_marker_path().exists());
    let record = restarted.read_activation_record().unwrap().unwrap();
    assert_eq!(record.activated_id, next.package_id());
    assert_eq!(record.previous_id.as_deref(), Some(old.package_id()));
    assert_eq!(record.phase, ActivationPhase::BootPending);
}

#[test]
fn startup_resolves_rollback_crashes_from_authoritative_pointer() {
    for committed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let cache = PackageCache::new(temp.path());
        cache.initialize().unwrap();
        let old = install_confirmed_base(&cache, temp.path());

        let mut manifest = sample_manifest();
        manifest.created_at = if committed {
            "2026-08-05T00:00:00Z".into()
        } else {
            "2026-08-06T00:00:00Z".into()
        };
        let mut next = stage_test_package(&cache, temp.path(), &manifest, "rollback-crash");
        Installer::activate(&mut next, &cache, &PlatformKind::Desktop).unwrap();
        let transaction = ActivationTransaction {
            version: ACTIVATION_FORMAT_VERSION,
            operation: TransactionOperation::Rollback,
            activated_id: next.package_id().to_string(),
            previous_id: Some(old.package_id().to_string()),
            moved_staged_to_active: false,
        };
        cache.write_transaction(&transaction).unwrap();
        if committed {
            cache.set_active_pointer(old.package_id()).unwrap();
        }

        let restarted = PackageCache::new(temp.path());
        restarted.initialize().unwrap();
        assert!(!restarted.transaction_path().unwrap().exists());
        let record = restarted.read_activation_record().unwrap().unwrap();
        if committed {
            assert_eq!(
                restarted.active_package_id().unwrap().as_deref(),
                Some(old.package_id())
            );
            assert_eq!(record.phase, ActivationPhase::RolledBack);
            assert!(!restarted.boot_marker_path().exists());
        } else {
            assert_eq!(
                restarted.active_package_id().unwrap().as_deref(),
                Some(next.package_id())
            );
            assert_eq!(record.phase, ActivationPhase::BootPending);
            assert!(restarted.boot_marker_path().exists());
        }
    }
}

#[test]
fn rollback_uses_record_even_with_multiple_unrelated_legacy_directories() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(temp.path());
    cache.initialize().unwrap();
    let old = install_confirmed_base(&cache, temp.path());

    let mut manifest = sample_manifest();
    manifest.created_at = "2026-08-04T00:00:00Z".into();
    let mut next = stage_test_package(&cache, temp.path(), &manifest, "rollback-next");
    Installer::activate(&mut next, &cache, &PlatformKind::Desktop).unwrap();
    std::fs::create_dir_all(temp.path().join("previous").join("a".repeat(64))).unwrap();
    std::fs::create_dir_all(temp.path().join("previous").join("b".repeat(64))).unwrap();

    let rolled_back = crate::rollback::RollbackManager::rollback(&cache).unwrap();
    assert_eq!(rolled_back.package_id(), old.package_id());
    assert_eq!(
        cache.active_package_id().unwrap().as_deref(),
        Some(old.package_id())
    );
}

#[test]
fn successful_boot_survives_restart_and_first_install_has_no_rollback_target() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(temp.path());
    cache.initialize().unwrap();
    let package = install_confirmed_base(&cache, temp.path());
    assert!(!crate::rollback::RollbackManager::needs_rollback(&cache));
    assert!(crate::rollback::RollbackManager::rollback(&cache).is_err());

    let restarted = PackageCache::new(temp.path());
    restarted.initialize().unwrap();
    assert_eq!(
        restarted.active_package_id().unwrap().as_deref(),
        Some(package.package_id())
    );
    assert!(!crate::rollback::RollbackManager::needs_rollback(
        &restarted
    ));
}

#[cfg(unix)]
#[test]
fn directory_copy_rejects_symlinks_before_creating_destination() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    let destination = tmp.path().join("destination");
    std::fs::create_dir_all(&source).unwrap();
    std::fs::write(tmp.path().join("outside.bin"), b"outside").unwrap();
    symlink(tmp.path().join("outside.bin"), source.join("link.bin")).unwrap();

    let error = copy_dir_all(&source, &destination).unwrap_err();
    assert!(matches!(error, UpdateError::UnsafePath { .. }));
    assert!(!destination.exists());
}
