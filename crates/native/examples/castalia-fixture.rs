// SPDX-License-Identifier: MIT OR Apache-2.0
//! Public synthetic acceptance producer/verifier. No production keys or network.
#[path = "../tests/fixtures/generic_castalia.rs"]
mod fixture;
#[path = "../tests/fixtures/social_receipts.rs"]
mod social_fixture;
use booknook_core::{
    authority::*,
    settings::{CastaliaSettings, ConnectionSettings},
    social_protocol::{self as social, Post, PostKind, SocialAction, SocialState},
};
use serde_json::{Value, json};
use std::io::Read;

fn saved_post_review(now: u64) -> Result<Value, booknook_core::Error> {
    let actor = social_fixture::actor(2);
    let original = social::prepare(
        &SocialState::default(),
        &actor,
        SocialAction::Publish {
            group_id: None,
            post: Post {
                kind: PostKind::Note,
                book: None,
                body: "Synthetic saved post".into(),
                stars: None,
                reading_status: None,
                spoiler: false,
                reply_to: None,
            },
        },
        now,
    )?;
    let mut edited = original.clone();
    let SocialAction::Publish { post, .. } = &mut edited.action else {
        unreachable!()
    };
    post.body = "Synthetic saved post revised: café".into();
    let (original_request, original_presentation, policy) = social_fixture::proof(&original, 2);
    let (edited_request, edited_presentation, _) = social_fixture::proof(&edited, 2);
    let pin = &policy.pins[0];
    let settings = ConnectionSettings {
        castalia: Some(CastaliaSettings {
            issuer: pin.issuer.clone(),
            key_id: pin.key_id.clone(),
            issuer_public_key: pin.public_key.clone(),
            audience: pin.audience.clone(),
            terminal_caller: policy.expected_caller.id,
            holder_public_key: actor,
            policy_digest: policy.policy_digest,
            credential_endpoint: "http://127.0.0.1:65500/credential".into(),
        }),
        ..Default::default()
    };
    Ok(json!({"settings": settings,
        "original": {"event": original, "request": original_request, "presentation": original_presentation},
        "edited": {"event": edited, "request": edited_request, "presentation": edited_presentation}}))
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let vector: serde_json::Value = serde_json::from_str(include_str!(
        "../tests/fixtures/castalia-custody-vector.json"
    ))?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs();
    let (request, expected, disclosure, policy) =
        fixture::fixture(vector["public_key"].as_str().unwrap(), now);
    if std::env::args().nth(1).as_deref() == Some("verify") {
        let mut input = String::new();
        std::io::stdin()
            .take(1_048_576)
            .read_to_string(&mut input)?;
        let value: serde_json::Value = booknook_core::strict_parse(input.as_bytes())?;
        let request = serde_json::to_vec(&value["request"])?;
        let presentation = serde_json::to_vec(&value["presentation"])?;
        let verified = verify_castalia_receipt(
            &request,
            &presentation,
            &policy,
            &expected,
            &disclosure,
            now,
        )?;
        println!(
            "{}",
            serde_json::json!({"verified":true,"holder_public_key":verified.holder_public_key})
        );
    } else {
        println!(
            "{}",
            serde_json::json!({"recovery":vector["container"],"passphrase":vector["passphrase"],"public_key":vector["public_key"],"request":serde_json::from_slice::<serde_json::Value>(&request)?,"expected_request":serde_json::from_slice::<serde_json::Value>(&expected)?,"disclosure":disclosure,"policy":policy,"saved_post_review":saved_post_review(now)?})
        );
    }
    Ok(())
}
