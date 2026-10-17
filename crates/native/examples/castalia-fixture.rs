// SPDX-License-Identifier: MIT OR Apache-2.0
//! Public synthetic acceptance producer/verifier. No production keys or network.
#[path = "../tests/fixtures/generic_castalia.rs"]
mod fixture;
use booknook_core::authority::*;
use std::io::Read;
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
            serde_json::json!({"recovery":vector["container"],"passphrase":vector["passphrase"],"public_key":vector["public_key"],"request":serde_json::from_slice::<serde_json::Value>(&request)?,"expected_request":serde_json::from_slice::<serde_json::Value>(&expected)?,"disclosure":disclosure,"policy":policy})
        );
    }
    Ok(())
}
