#[test]
fn verify_payload_hashes_all_match() {
    let mut manifest = sample_manifest();
    let data = b"hello payload";
    let hash: HashDigest = Sha256::digest(data).into();

    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "patch.bundle".into(),
        algorithm: "sha256".into(),
        hash,
    }];

    let dir = std::env::temp_dir().join("verify_hash_ok");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    create_temp_payload(&dir, "patch.bundle", data);

    assert!(Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_payload_hashes_ignores_missing_other_platform_but_checks_all() {
    let mut manifest = sample_manifest();
    let desktop_data = b"desktop";
    let common_data = b"common";
    manifest.payload_hashes = vec![
        PayloadHash {
            platform: PlatformKind::Desktop,
            path: "desktop.bin".into(),
            algorithm: "sha256".into(),
            hash: Sha256::digest(desktop_data).into(),
        },
        PayloadHash {
            platform: PlatformKind::Android,
            path: "android-missing.bin".into(),
            algorithm: "sha256".into(),
            hash: [7; 32],
        },
        PayloadHash {
            platform: PlatformKind::All,
            path: "common.bin".into(),
            algorithm: "sha256".into(),
            hash: Sha256::digest(common_data).into(),
        },
    ];
    let temp = tempfile::tempdir().unwrap();
    create_temp_payload(temp.path(), "desktop.bin", desktop_data);
    create_temp_payload(temp.path(), "common.bin", common_data);

    assert!(
        Verifier::verify_payload_hashes(&manifest, temp.path(), &PlatformKind::Desktop,).is_ok()
    );
    assert!(
        Verifier::verify_payload_hashes(&manifest, temp.path(), &PlatformKind::Android,).is_err()
    );
}

#[test]
fn verify_payload_hashes_mismatch() {
    let mut manifest = sample_manifest();
    let data = b"hello payload";
    let hash: HashDigest = Sha256::digest(data).into();

    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "patch.bundle".into(),
        algorithm: "sha256".into(),
        hash,
    }];

    let dir = std::env::temp_dir().join("verify_hash_bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    create_temp_payload(&dir, "patch.bundle", b"tampered data");

    let result = Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_payload_hashes_missing_file() {
    let mut manifest = sample_manifest();
    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "missing.bundle".into(),
        algorithm: "sha256".into(),
        hash: [0u8; 32],
    }];

    let dir = std::env::temp_dir().join("verify_hash_miss");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let result = Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_payload_hashes_multiple_errors() {
    let mut manifest = sample_manifest();
    manifest.payload_hashes = vec![
        PayloadHash {
            platform: PlatformKind::Desktop,
            path: "a.bundle".into(),
            algorithm: "sha256".into(),
            hash: [1u8; 32],
        },
        PayloadHash {
            platform: PlatformKind::Desktop,
            path: "b.bundle".into(),
            algorithm: "sha256".into(),
            hash: [2u8; 32],
        },
    ];

    let dir = std::env::temp_dir().join("verify_hash_multi");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    create_temp_payload(&dir, "a.bundle", b"data");

    let result = Verifier::verify_payload_hashes(&manifest, &dir, &PlatformKind::Desktop);
    assert!(result.is_err());
    // Should have at least one error (b.bundle missing)
    assert!(!result.unwrap_err().is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_rejects_payload_traversal_before_reading_outside_stage() {
    let mut manifest = sample_manifest();
    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "../outside.bin".into(),
        algorithm: "sha256".into(),
        hash: [0u8; 32],
    }];
    let temp = tempfile::tempdir().unwrap();
    let staged = temp.path().join("staged");
    std::fs::create_dir_all(&staged).unwrap();
    std::fs::write(temp.path().join("outside.bin"), b"outside").unwrap();

    let errors =
        Verifier::verify_payload_hashes(&manifest, &staged, &PlatformKind::Desktop).unwrap_err();
    assert!(matches!(errors[0], UpdateError::UnsafePath { .. }));
}

// ── Compatibility tests ─────────────────────────────────────────────
