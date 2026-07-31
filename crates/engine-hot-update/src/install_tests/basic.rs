#[test]
fn installer_stage_moves_files() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let manifest = sample_manifest();
    let staging_dir = tmp.path().join("download_temp");
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::write(staging_dir.join("data.bin"), b"test payload").unwrap();

    let pkg = Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
    assert_eq!(pkg.state, PackageState::Staged);
    assert!(pkg.staging_dir().join("data.bin").exists());
}

#[test]
fn installer_stage_persists_state() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let manifest = sample_manifest();
    let staging_dir = tmp.path().join("download_temp2");
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::write(staging_dir.join("data.bin"), b"test").unwrap();

    let pkg = Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();

    // Verify state was persisted.
    let loaded = cache.get_package(pkg.package_id()).unwrap();
    assert_eq!(loaded.state, PackageState::Staged);
}

#[test]
fn installer_stage_only_contains_current_platform_and_all_payloads() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();
    let mut manifest = sample_manifest();
    manifest.payload_hashes = vec![
        PayloadHash {
            platform: PlatformKind::Desktop,
            path: "desktop.bin".into(),
            algorithm: "sha256".into(),
            hash: [1; 32],
        },
        PayloadHash {
            platform: PlatformKind::Android,
            path: "android.bin".into(),
            algorithm: "sha256".into(),
            hash: [2; 32],
        },
        PayloadHash {
            platform: PlatformKind::All,
            path: "common.bin".into(),
            algorithm: "sha256".into(),
            hash: [3; 32],
        },
    ];
    let download = tmp.path().join("platform-download");
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(download.join("desktop.bin"), b"desktop").unwrap();
    std::fs::write(download.join("common.bin"), b"common").unwrap();
    // android.bin is deliberately absent and must not be required.

    let package = Installer::stage(&manifest, &download, &cache, &PlatformKind::Desktop).unwrap();

    assert!(package.staging_dir().join("desktop.bin").is_file());
    assert!(package.staging_dir().join("common.bin").is_file());
    assert!(!package.staging_dir().join("android.bin").exists());
}

#[test]
fn installer_activate_switches_active() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let manifest = sample_manifest();
    let staging_dir = tmp.path().join("download_act");
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::write(staging_dir.join("data.bin"), b"activate me").unwrap();

    let mut pkg =
        Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut pkg, &cache, &PlatformKind::Desktop).unwrap();

    assert_eq!(pkg.state, PackageState::Active);
    assert!(pkg.active_dir().join("data.bin").exists());

    // Active pointer should point to this package.
    let active = cache.active_package().unwrap();
    assert_eq!(active.package_id(), pkg.package_id());
}

#[test]
fn installer_activate_creates_boot_marker() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let manifest = sample_manifest();
    let staging_dir = tmp.path().join("download_boot");
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::write(staging_dir.join("data.bin"), b"boot").unwrap();

    let mut pkg =
        Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut pkg, &cache, &PlatformKind::Desktop).unwrap();

    assert!(cache.boot_marker_path().exists());
}

#[test]
fn installer_activate_fails_without_staged_dir() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let manifest = sample_manifest();
    let mut pkg = Package::new(manifest, tmp.path());

    let result = Installer::activate(&mut pkg, &cache, &PlatformKind::Desktop);
    assert!(result.is_err());
    assert!(matches!(result, Err(UpdateError::ActivationFailed(_))));
}

#[test]
fn installer_activate_preserves_previous() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    // Create and activate first package.
    let m1 = sample_manifest();
    let s1 = tmp.path().join("dl_first");
    std::fs::create_dir_all(&s1).unwrap();
    std::fs::write(s1.join("data.bin"), b"first").unwrap();
    let mut pkg1 = Installer::stage(&m1, &s1, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut pkg1, &cache, &PlatformKind::Desktop).unwrap();
    Installer::mark_boot_successful(&cache).unwrap();

    // Create and activate second package.
    let mut m2 = sample_manifest();
    m2.created_at = "2026-06-01T00:00:00Z".into();
    let s2 = tmp.path().join("dl_second");
    std::fs::create_dir_all(&s2).unwrap();
    std::fs::write(s2.join("data.bin"), b"second").unwrap();
    let mut pkg2 = Installer::stage(&m2, &s2, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut pkg2, &cache, &PlatformKind::Desktop).unwrap();

    // Activated payloads are immutable and retained under active/<id>.
    let prev_path = cache.base_dir.join("active").join(pkg1.package_id());
    let prev_content = std::fs::read(prev_path.join("data.bin")).unwrap();
    assert_eq!(prev_content, b"first");
}

#[test]
fn installer_mark_failed_boot_creates_fail_flag() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let manifest = sample_manifest();
    let staging_dir = tmp.path().join("download_failed_boot");
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::write(staging_dir.join("data.bin"), b"boot").unwrap();
    let mut package =
        Installer::stage(&manifest, &staging_dir, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut package, &cache, &PlatformKind::Desktop).unwrap();

    Installer::mark_failed_boot(&cache).unwrap();

    // Both records remain so restart has deterministic rollback metadata.
    assert!(cache.boot_marker_path().exists());
    assert!(tmp.path().join("boot_failed").exists());
}

#[test]
fn installer_activate_replaces_old_active() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    // First package.
    let m1 = sample_manifest();
    let s1 = tmp.path().join("dl_rep1");
    std::fs::create_dir_all(&s1).unwrap();
    std::fs::write(s1.join("data.bin"), b"v1").unwrap();
    let mut p1 = Installer::stage(&m1, &s1, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut p1, &cache, &PlatformKind::Desktop).unwrap();
    let id1 = p1.package_id().to_string();
    Installer::mark_boot_successful(&cache).unwrap();

    // Second package.
    let mut m2 = sample_manifest();
    m2.created_at = "2026-07-01T00:00:00Z".into();
    let s2 = tmp.path().join("dl_rep2");
    std::fs::create_dir_all(&s2).unwrap();
    std::fs::write(s2.join("data.bin"), b"v2").unwrap();
    let mut p2 = Installer::stage(&m2, &s2, &cache, &PlatformKind::Desktop).unwrap();
    Installer::activate(&mut p2, &cache, &PlatformKind::Desktop).unwrap();

    // Active pointer should now point to p2.
    let active = cache.active_package().unwrap();
    assert_eq!(active.package_id(), p2.package_id());

    // p1 remains immutable in active so pointer rollback needs no move.
    let p1_active = cache.base_dir.join("active").join(&id1);
    assert!(p1_active.exists());
}

#[test]
fn installer_rejects_manifest_before_replacing_existing_stage() {
    let tmp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(tmp.path());
    cache.initialize().unwrap();

    let mut manifest = sample_manifest();
    manifest.payload_hashes[0].path = "../escape.bin".into();
    let package = Package::new(manifest.clone(), tmp.path());
    let existing_stage = package.staging_dir();
    std::fs::create_dir_all(&existing_stage).unwrap();
    let sentinel = existing_stage.join("sentinel.txt");
    std::fs::write(&sentinel, b"keep").unwrap();

    let download = tmp.path().join("download-malicious");
    std::fs::create_dir_all(&download).unwrap();
    std::fs::write(download.join("safe.bin"), b"data").unwrap();

    let error = Installer::stage(&manifest, &download, &cache, &PlatformKind::Desktop).unwrap_err();
    assert!(matches!(error, UpdateError::UnsafePath { .. }));
    assert_eq!(std::fs::read(sentinel).unwrap(), b"keep");
    assert!(download.exists());
}

#[test]
fn activation_rejects_pending_and_failed_previous_until_boot_is_confirmed() {
    let temp = tempfile::tempdir().unwrap();
    let cache = PackageCache::new(temp.path());
    cache.initialize().unwrap();

    let first_manifest = sample_manifest();
    let mut first = stage_test_package(&cache, temp.path(), &first_manifest, "known-good-first");
    Installer::activate(&mut first, &cache, &PlatformKind::Desktop).unwrap();

    let mut second_manifest = sample_manifest();
    second_manifest.created_at = "2026-07-15T00:00:00Z".into();
    let mut second = stage_test_package(&cache, temp.path(), &second_manifest, "known-good-second");

    let pending_error =
        Installer::activate(&mut second, &cache, &PlatformKind::Desktop).unwrap_err();
    assert!(matches!(pending_error, UpdateError::ActivationFailed(_)));
    assert_eq!(
        cache.active_package_id().unwrap().as_deref(),
        Some(first.package_id())
    );

    Installer::mark_failed_boot(&cache).unwrap();
    let failed_error =
        Installer::activate(&mut second, &cache, &PlatformKind::Desktop).unwrap_err();
    assert!(matches!(failed_error, UpdateError::ActivationFailed(_)));

    Installer::mark_boot_successful(&cache).unwrap();
    Installer::activate(&mut second, &cache, &PlatformKind::Desktop).unwrap();
    assert_eq!(
        cache.active_package_id().unwrap().as_deref(),
        Some(second.package_id())
    );
}
