#[test]
fn verify_cooked_headers_skips_non_cooked() {
    let mut manifest = sample_manifest();
    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "data.bin".into(),
        algorithm: "sha256".into(),
        hash: [0u8; 32],
    }];

    let dir = std::env::temp_dir().join("verify_cooked_skip");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    create_temp_payload(&dir, "data.bin", b"not a cooked file");

    // Should pass because we skip non-.cooked files.
    assert!(Verifier::verify_cooked_headers(&manifest, &dir, &PlatformKind::Desktop).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_cooked_headers_valid() {
    use engine_asset::cook::write_cooked_artifact;
    use engine_serialize::SchemaVersion;

    let mut manifest = sample_manifest();
    let hash: HashDigest = Sha256::digest(b"payload data").into();
    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "asset.cooked".into(),
        algorithm: "sha256".into(),
        hash,
    }];

    let dir = std::env::temp_dir().join("verify_cooked_ok");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Write a valid cooked artifact.
    write_cooked_artifact(
        &dir.join("asset.cooked"),
        1,
        b"payload data",
        SchemaVersion::new(0, 1, 0),
    )
    .unwrap();

    assert!(Verifier::verify_cooked_headers(&manifest, &dir, &PlatformKind::Desktop).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_cooked_headers_invalid_magic() {
    let mut manifest = sample_manifest();
    let hash: HashDigest = Sha256::digest(b"bad data").into();
    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "bad.cooked".into(),
        algorithm: "sha256".into(),
        hash,
    }];

    let dir = std::env::temp_dir().join("verify_cooked_bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // Write garbage instead of a valid cooked file.
    create_temp_payload(&dir, "bad.cooked", b"garbage data");

    let result = Verifier::verify_cooked_headers(&manifest, &dir, &PlatformKind::Desktop);
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

// ── Full verify tests ───────────────────────────────────────────────

#[test]
fn verify_full_pipeline_accepts_valid_package() {
    use engine_asset::cook::write_cooked_artifact;
    use engine_serialize::SchemaVersion;
    use sha2::{Digest, Sha256};

    let mut manifest = sample_manifest();
    let payload_data = b"cooked content";

    let dir = std::env::temp_dir().join("verify_full_ok");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    write_cooked_artifact(
        &dir.join("mesh.cooked"),
        1,
        payload_data,
        SchemaVersion::new(0, 1, 0),
    )
    .unwrap();

    // Hash must be computed from the entire written file (header + payload).
    let file_data = std::fs::read(dir.join("mesh.cooked")).unwrap();
    let hash: HashDigest = Sha256::digest(&file_data).into();

    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "mesh.cooked".into(),
        algorithm: "sha256".into(),
        hash,
    }];

    let result =
        Verifier::development().verify(&manifest, &dir, &PlatformKind::Desktop, "1.5.0", (1, 5));
    assert!(result.is_ok(), "expected Ok, got: {result:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verify_full_pipeline_rejects_bad_hash() {
    let mut manifest = sample_manifest();
    manifest.payload_hashes = vec![PayloadHash {
        platform: PlatformKind::Desktop,
        path: "data.bin".into(),
        algorithm: "sha256".into(),
        hash: [0xAA; 32],
    }];

    let dir = std::env::temp_dir().join("verify_full_bad");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    create_temp_payload(&dir, "data.bin", b"does not match");

    let result =
        Verifier::development().verify(&manifest, &dir, &PlatformKind::Desktop, "1.5.0", (1, 5));
    assert!(result.is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
