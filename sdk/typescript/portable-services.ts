// SPDX-License-Identifier: MIT OR Apache-2.0
// Portable service wire types; checked against actual native/WASM outputs by SDK parity tests.
export interface CatalogCandidate { book_id: string; title: string; subjects: string[] }
export interface SharedRating { actor: string; book_id: string; stars: number }
/** Deliberately has no private notebook, annotation, card or handwriting inputs. */
export interface RecommendationInput {
  candidates: CatalogCandidate[];
  preferred_subjects: string[];
  shared_ratings: SharedRating[];
  friends: string[];
  excluded: string[];
}
export interface Recommendation { book_id: string; score: number; reasons: string[] }
export interface SyncBatch {
  schema: "booknook.sync-batch.v1";
  id: string;
  actor: string;
  revision: number;
  recipients: string[];
  content_digest: string;
}
export interface EncryptedSyncBatch { batch: SyncBatch; nonce: string; ciphertext: string }
/** The existing native transfer-file wire. It contains no symmetric or signing key. */
export interface ApprovedSyncPacket {
  schema: "booknook.sync-packet.v1";
  envelope: EncryptedSyncBatch;
  approved_at: number;
  request_json: string;
  presentation_json: string;
}
export interface SealedSyncBatch {
  packet: ApprovedSyncPacket;
  /** Consume atomically with storing the outbox; do not transmit before that commit. */
  replay_key: string;
}
export interface OpenedSyncBatch {
  plaintext: Uint8Array;
  /** Store with packet.envelope.batch.id atomically when applying the verified content. */
  packet_digest: string;
  replay_key: string;
}
