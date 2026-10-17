// SPDX-License-Identifier: MIT OR Apache-2.0
use booknook_native::Desk;

#[test]
fn candidate_validation_neither_unlocks_nor_replaces_live_custody() {
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/castalia-custody-vector.json")).unwrap();
    let recovery = serde_json::to_string(&vector["container"]).unwrap();
    let passphrase = vector["passphrase"].as_str().unwrap().to_owned();
    let expected = vector["public_key"].as_str().unwrap().to_owned();
    let root = std::env::temp_dir().join(format!(
        "booknook-recovery-validation-{}",
        booknook_core::new_id()
    ));
    let desk = Desk::new(root.to_string_lossy().into_owned()).unwrap();
    assert_eq!(
        desk.custody_validate_recovery(recovery.clone(), passphrase.clone())
            .unwrap(),
        expected
    );
    assert_eq!(desk.custody_public_key().unwrap(), None);
    desk.custody_unlock(recovery.clone(), passphrase.clone())
        .unwrap();
    assert!(
        desk.custody_validate_recovery(recovery.clone(), "wrong synthetic passphrase".into())
            .is_err()
    );
    assert_eq!(
        desk.custody_public_key().unwrap().as_deref(),
        Some(expected.as_str())
    );
    assert!(
        desk.custody_validate_recovery("{}".into(), passphrase.clone())
            .is_err()
    );
    assert_eq!(
        desk.custody_public_key().unwrap().as_deref(),
        Some(expected.as_str())
    );
    assert_eq!(
        desk.custody_validate_recovery(recovery, passphrase)
            .unwrap(),
        expected
    );
    assert_eq!(
        desk.custody_public_key().unwrap().as_deref(),
        Some(expected.as_str())
    );
    drop(desk);
    std::fs::remove_dir_all(root).unwrap();
}
