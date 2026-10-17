// SPDX-License-Identifier: MIT OR Apache-2.0
//! Optional operator-configured issuer. Never creates identity or grants.
#[path = "../reference_issuer.rs"]
mod reference_issuer;
fn main() {
    if let Err(code) = reference_issuer::run() {
        eprintln!("{}", code.code());
        std::process::exit(code.exit());
    }
}
