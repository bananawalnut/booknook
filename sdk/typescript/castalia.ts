// SPDX-License-Identifier: MIT OR Apache-2.0
import type {Caller, TrustPin} from "./social-types.js";
import {checkTime, encodeContract, parseContract} from "./contract-json.js";

/** Independently configured by the host, never inferred from an incoming credential. */
export interface CastaliaTrustPolicy {
  pins: TrustPin[];
  expected_caller: Caller;
  audience: string;
  holder_public_key: string;
  policy_digest: string;
}
export interface CastaliaDisclosure { title: string; statements: string[] }
export interface CastaliaPresentationRequest {
  schema: "castalia.credential-presentation-request.v2";
  request_bytes_base64: string;
  disclosure: CastaliaDisclosure;
  credential: {
    claims: {
      schema: "castalia.request-credential.v1"; issuer: string; key_id: string;
      holder_public_key: string; audience: string; caller: Caller;
      request_digest_sha256: string; disclosure_digest_sha256: string;
      policy_digest_sha256: string; nonce: string; issued_at: number; expires_at: number;
    };
    signature: string;
  };
}
export interface VerifiedCastaliaPresentation {
  holder_public_key: string;
  request_digest_sha256: string;
  expires_at: number;
  replay_key: string;
}
/** Verification APIs only; the WASM interface never accepts a private signing key. */
export interface CastaliaVerificationCore {
  validate_castalia_request(request: string, expectedRequest: string, expectedDisclosure: string, policy: string, now: bigint): string;
  verify_castalia_receipt(request: string, presentation: string, expectedRequest: string, expectedDisclosure: string, policy: string, now: bigint): string;
}

/** expectedRequest is the host's original canonical JSON, never re-encoded in JavaScript. */
export function validateCastaliaRequest(core: CastaliaVerificationCore, request: string, expectedRequest: string,
  disclosure: CastaliaDisclosure, policy: CastaliaTrustPolicy, now: bigint): CastaliaPresentationRequest {
  checkTime(now);
  return parseContract(core.validate_castalia_request(request, expectedRequest, encodeContract(disclosure), encodeContract(policy), now));
}

/** This result establishes neither application permission nor replay consumption. Validate
 * your application's exact action and atomically persist replay_key with its effects. */
export function verifyCastaliaReceipt(core: CastaliaVerificationCore, request: string, presentation: string,
  expectedRequest: string, disclosure: CastaliaDisclosure, policy: CastaliaTrustPolicy, now: bigint): VerifiedCastaliaPresentation {
  checkTime(now);
  return parseContract(core.verify_castalia_receipt(request, presentation, expectedRequest, encodeContract(disclosure), encodeContract(policy), now));
}
