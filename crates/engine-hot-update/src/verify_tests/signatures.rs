#[test]
fn unsigned_manifest_is_rejected_in_production() {
    let error = Verifier::production()
        .verify_signature(&sample_manifest())
        .unwrap_err();
    assert!(matches!(error, UpdateError::SignatureMissing));
}

#[test]
fn unsigned_manifest_is_accepted_only_by_explicit_relaxed_policies() {
    let manifest = sample_manifest();
    assert!(Verifier::development().verify_signature(&manifest).is_ok());
    assert!(Verifier::new(SignaturePolicy::AllowUnsigned)
        .verify_signature(&manifest)
        .is_ok());
}

#[test]
fn valid_ed25519_signature_is_accepted() {
    let (manifest, verifier) = signed_manifest("release-2026");
    assert!(verifier.verify_signature(&manifest).is_ok());
}

#[test]
fn tampered_manifest_is_rejected() {
    let (mut manifest, verifier) = signed_manifest("release-2026");
    manifest.engine_version = "1.5.1".into();
    let error = verifier.verify_signature(&manifest).unwrap_err();
    assert!(matches!(error, UpdateError::SignatureInvalid { .. }));
}

#[test]
fn signature_from_wrong_key_is_rejected() {
    let (manifest, _) = signed_manifest("release-2026");
    let (_, wrong_public_key) = signing_key();
    let verifier = Verifier::production()
        .with_trusted_ed25519_key("release-2026", &wrong_public_key)
        .unwrap();
    let error = verifier.verify_signature(&manifest).unwrap_err();
    assert!(matches!(error, UpdateError::SignatureInvalid { .. }));
}

#[test]
fn unknown_key_id_is_rejected() {
    let (manifest, _) = signed_manifest("release-2026");
    let error = Verifier::production()
        .verify_signature(&manifest)
        .unwrap_err();
    assert!(matches!(
        error,
        UpdateError::SignatureUnknownKey { ref key_id } if key_id == "release-2026"
    ));
}

#[test]
fn bad_signature_is_rejected_even_in_development() {
    let (mut manifest, production) = signed_manifest("release-2026");
    manifest.signature.as_mut().unwrap().value[0] ^= 0xff;
    let mut development = Verifier::development();
    development
        .trust_ed25519_key(
            "release-2026",
            &production.trusted_ed25519_keys["release-2026"],
        )
        .unwrap();
    let error = development.verify_signature(&manifest).unwrap_err();
    assert!(matches!(error, UpdateError::SignatureInvalid { .. }));
}

#[test]
fn unsupported_algorithm_is_rejected() {
    let mut manifest = sample_manifest();
    manifest.signature = Some(ManifestSignature {
        algorithm: "rsa-sha256".into(),
        value: vec![0u8; 256],
        key_id: "key-02".into(),
        signed_at: "2026-05-29T12:00:00Z".into(),
    });
    let error = Verifier::development()
        .verify_signature(&manifest)
        .unwrap_err();
    assert!(matches!(
        error,
        UpdateError::SignatureUnsupportedAlgorithm { ref algorithm }
            if algorithm == "rsa-sha256"
    ));
}

#[test]
fn canonical_bytes_are_stable_and_ignore_signature_metadata() {
    let mut manifest = sample_manifest();
    let first = canonical_manifest_bytes(&manifest).unwrap();
    let second = canonical_manifest_bytes(&manifest).unwrap();
    assert_eq!(first, second);
    assert!(first.starts_with(MANIFEST_SIGNATURE_DOMAIN_V2));
    assert_eq!(
        crate::package::hex_encode(&Sha256::digest(&first)),
        "64de56d94b403f7cd9f15ea686ce47874c09dca033a92877c1d174419ca29d67"
    );

    manifest.signature = Some(ManifestSignature {
        algorithm: "anything".into(),
        value: vec![1, 2, 3],
        key_id: "anything".into(),
        signed_at: "anything".into(),
    });
    assert_eq!(first, canonical_manifest_bytes(&manifest).unwrap());
}

// ── Payload hash tests ──────────────────────────────────────────────
