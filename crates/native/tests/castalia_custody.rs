// SPDX-License-Identifier: MIT OR Apache-2.0
#[path = "fixtures/generic_castalia.rs"]
mod fixture;
use booknook_core::{authority::*, canonical};
use booknook_native::CastaliaCustody;

#[test]
fn standalone_public_object_approves_second_application_without_a_desk() {
    let vector: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/castalia-custody-vector.json")).unwrap();
    let recovery = serde_json::to_string(&vector["container"]).unwrap();
    let pass = vector["passphrase"].as_str().unwrap().to_owned();
    let holder = vector["public_key"].as_str().unwrap();
    let custody = CastaliaCustody::new();
    assert_eq!(
        custody
            .validate_recovery(recovery.clone(), pass.clone())
            .unwrap(),
        holder
    );
    assert!(custody.public_key().unwrap().is_none());
    assert_eq!(custody.unlock(recovery, pass).unwrap(), holder);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (request, expected, disclosure, policy) = fixture::fixture(holder, now);
    let prepare = || {
        custody.prepare(
            String::from_utf8(request.clone()).unwrap(),
            String::from_utf8(expected.clone()).unwrap(),
            serde_json::to_string(&disclosure).unwrap(),
            serde_json::to_string(&policy).unwrap(),
        )
    };
    let prepared: serde_json::Value = serde_json::from_str(&prepare().unwrap()).unwrap();
    let token = prepared["token"].as_str().unwrap().to_owned();
    let receipt = custody
        .approve(token.clone(), serde_json::to_string(&policy).unwrap())
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let verified = verify_castalia_receipt(
        &request,
        receipt.as_bytes(),
        &policy,
        &expected,
        &disclosure,
        now,
    )
    .unwrap();
    assert_eq!(verified.holder_public_key, holder);
    assert!(verified.replay_key().starts_with(holder));
    assert!(
        custody
            .approve(token, serde_json::to_string(&policy).unwrap())
            .is_err()
    );
    let book_policy = AuthorityPolicy {
        pins: policy.pins.clone(),
        expected_caller: policy.expected_caller.clone(),
        audience: policy.audience.clone(),
        actor: holder.into(),
        action: "sync.send".into(),
        policy_digest: policy.policy_digest.clone(),
    };
    assert!(
        verify_receipt(
            &request,
            receipt.as_bytes(),
            &book_policy,
            b"anything",
            3,
            now
        )
        .is_err(),
        "Generic approval must not bypass Book Nook action validation"
    );
    let mut changed = disclosure.clone();
    changed.title = "A different disclosure".into();
    assert!(
        custody
            .prepare(
                String::from_utf8(request.clone()).unwrap(),
                String::from_utf8(expected.clone()).unwrap(),
                serde_json::to_string(&changed).unwrap(),
                serde_json::to_string(&policy).unwrap()
            )
            .is_err()
    );
    let mut changed = policy.clone();
    changed.pins[0].public_key = "00".repeat(32);
    assert!(
        custody
            .prepare(
                String::from_utf8(request.clone()).unwrap(),
                String::from_utf8(expected.clone()).unwrap(),
                serde_json::to_string(&disclosure).unwrap(),
                serde_json::to_string(&changed).unwrap()
            )
            .is_err()
    );
    let prepared: serde_json::Value = serde_json::from_str(&prepare().unwrap()).unwrap();
    let token = prepared["token"].as_str().unwrap().to_owned();
    assert!(custody.approve(token.clone(), "{}".into()).is_err());
    assert!(
        custody
            .approve(token, serde_json::to_string(&policy).unwrap())
            .is_err(),
        "Malformed current policy must consume approval"
    );
    let prepared: serde_json::Value = serde_json::from_str(&prepare().unwrap()).unwrap();
    custody
        .cancel(prepared["token"].as_str().unwrap().into())
        .unwrap();
    assert!(
        custody
            .approve(
                prepared["token"].as_str().unwrap().into(),
                serde_json::to_string(&policy).unwrap()
            )
            .is_err()
    );
    let prepared: serde_json::Value = serde_json::from_str(&prepare().unwrap()).unwrap();
    custody.lock().unwrap();
    assert!(
        custody
            .approve(
                prepared["token"].as_str().unwrap().into(),
                serde_json::to_string(&policy).unwrap()
            )
            .is_err()
    );
    assert!(custody.public_key().unwrap().is_none());
    assert!(
        custody
            .canonical_request("{\"schema\":1,\"schema\":2}".into())
            .is_err()
    );
    assert_eq!(
        custody
            .canonical_request("{\"z\":1, \"a\":2}".into())
            .unwrap(),
        String::from_utf8(canonical(&serde_json::json!({"a":2,"z":1})).unwrap()).unwrap()
    );
}
