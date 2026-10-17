// SPDX-License-Identifier: MIT OR Apache-2.0
import type { State, Command, ProviderConnection, NativeAuthorityProfile } from "./types.js";
export type * from "./types.js";
export type * from "./portable-services.js";
export * from "./castalia.js";
export * from "./providers.js";
export * from "./archives.js";

import {parseContract, encodeContract, checkTime, checkNumbers} from "./contract-json.js";

export interface WasmCore {
  new_state(): string;
  execute(state: string, command: string): string;
  validate_archive(state: string): void;
}
/** Public Castalia configuration only; validation does not establish trust or sign in. */
export interface CastaliaConfigurationCore {
  validate_provider_connections(connectionsJson: string): string;
  validate_native_authority_profile(profileJson: string): string;
}
/** Pass original JSON so the shared parser can reject duplicate and unknown fields. */
export function validateProviderConnections(core: CastaliaConfigurationCore, rawJson: string): ProviderConnection[] {
  return parseContract(core.validate_provider_connections(rawJson));
}
/** The host must separately review and explicitly install a proposed issuer policy. */
export function validateNativeAuthorityProfile(core: CastaliaConfigurationCore, rawJson: string): NativeAuthorityProfile {
  return parseContract(core.validate_native_authority_profile(rawJson));
}
export interface NotebookStorage {
  load(): Promise<State | null>;
  /** Must be atomic. null means create only if no record exists. */
  compareAndSave(expectedRevision: number | null, state: State): Promise<boolean>;
}
export class BookNook {
  private constructor(private readonly core: WasmCore, private readonly storage: NotebookStorage) {}
  static async open(core: WasmCore, storage: NotebookStorage): Promise<BookNook> {
    let state = await storage.load();
    if (!state) {
      state = parseContract<State>(core.new_state());
      if (!(await storage.compareAndSave(null, state))) state = await storage.load();
    }
    if (!state) throw new Error("Storage did not retain the initial state");
    core.validate_archive(encodeContract(state));
    return new BookNook(core, storage);
  }
  async execute(command: Command): Promise<{state: State; result: unknown}> {
    const state = await this.storage.load();
    if (!state) throw new Error("Notebook storage unavailable");
    const response = parseContract<{state: State; result: unknown}>(this.core.execute(encodeContract(state), encodeContract(command)));
    if (!(await this.storage.compareAndSave(state.revision, response.state))) throw new Error("Storage conflict: reload before retrying");
    return response;
  }
}

/** The portable social contract is a separate namespace from private notebook records. */
export type * as Social from "./social-types.js";
import type { SocialState, SocialAction, LocalSocialCommand, SocialEvent, ApprovedPublication, AuthorityPolicy } from "./social-types.js";
export interface SyncConflict {
  id: string; collection: string; record_id: string; field: string; local: unknown; incoming: unknown;
}
export interface PortableCore extends WasmCore {
  reconcile_states(base: string, local: string, incoming: string): string;
  resolve_conflict(state: string, conflict: string, useIncoming: boolean): string;
  new_social_state(): string;
  social_local(state: string, command: string): string;
  social_prepare(state: string, actor: string, action: string, now: bigint): string;
  social_approve(state: string, event: string, request: string, presentation: string, policy: string, now: bigint): string;
  social_receive(state: string, publication: string, policy: string, now: bigint): string;
  social_feed(state: string, viewer: string, revealSpoilers: boolean): string;
}
export function reconcile(core: PortableCore, base: State | null, local: State, incoming: State): {state: State; conflicts: SyncConflict[]} {
  return parseContract(core.reconcile_states(base ? encodeContract(base) : "", encodeContract(local), encodeContract(incoming)));
}
export function resolveConflict(core: PortableCore, state: State, conflict: SyncConflict, useIncoming: boolean): State {
  return parseContract(core.resolve_conflict(encodeContract(state), encodeContract(conflict), useIncoming));
}
export interface SocialStorage {
  load(): Promise<SocialState | null>;
  /** Approval, replay consumption and durable outbox must commit together before transport. */
  compareAndSave(expectedRevision: number | null, state: SocialState): Promise<boolean>;
}
/** Hosts provide custody, their independently pinned issuer policy and Nostr transport. */
export class BookNookSocial {
  constructor(private readonly core: PortableCore, private readonly storage: SocialStorage) {}
  async state(): Promise<SocialState> {
    let state = await this.storage.load();
    if (!state) {
      const initial = parseContract<SocialState>(this.core.new_social_state());
      if (await this.storage.compareAndSave(null, initial)) state = initial;
      else state = await this.storage.load();
    }
    if (!state) throw new Error("Social storage unavailable");
    checkNumbers(state); return state;
  }
  private async save(expected: number, state: SocialState): Promise<void> {
    if (!(await this.storage.compareAndSave(expected, state))) throw new Error("Social store changed; review the action again");
  }
  async local(command: LocalSocialCommand): Promise<SocialState> {
    const before = await this.state();
    const state = parseContract<SocialState>(this.core.social_local(encodeContract(before), encodeContract(command)));
    await this.save(before.revision, state); return state;
  }
  async prepare(actor: string, action: SocialAction, now: bigint): Promise<SocialEvent> {
    checkTime(now);
    return parseContract(this.core.social_prepare(encodeContract(await this.state()), actor, encodeContract(action), now));
  }
  async approve(event: SocialEvent, request: string, presentation: string, policy: AuthorityPolicy, now: bigint): Promise<ApprovedPublication> {
    checkTime(now);
    const before = await this.state();
    const result = parseContract<{state: SocialState; publication: ApprovedPublication}>(this.core.social_approve(encodeContract(before), encodeContract(event), request, presentation, encodeContract(policy), now));
    await this.save(before.revision, result.state); return result.publication;
  }
  async receive(publication: ApprovedPublication, policy: AuthorityPolicy, now: bigint): Promise<boolean> {
    checkTime(now);
    const before = await this.state();
    const result = parseContract<{state: SocialState; changed: boolean}>(this.core.social_receive(encodeContract(before), encodeContract(publication), encodeContract(policy), now));
    if (result.changed) await this.save(before.revision, result.state);
    return result.changed;
  }
}

import type { AudioObservation, AudioEditionCandidates, ProgressObservation } from "./types.js";
export interface AudioMatchingCore {
  audio_edition_candidates(state: string, observation: string, now: bigint): string;
  reviewed_audio_observation(state: string, observation: string, passID: string, confirmedEditionID: string | undefined, now: bigint): string;
}
/** Suggestions never choose an edition or translate audio time into ebook progress. */
export function audioEditionCandidates(core: AudioMatchingCore, state: State, observation: AudioObservation, now: bigint): AudioEditionCandidates {
  checkTime(now);
  return parseContract(core.audio_edition_candidates(encodeContract(state), encodeContract(observation), now));
}
/** Host obtains explicit review before confirming; persist through the normal ObservePosition command. */
export function reviewedAudioObservation(core: AudioMatchingCore, state: State, observation: AudioObservation, passID: string, confirmedEditionID: string | undefined, now: bigint): ProgressObservation {
  checkTime(now);
  return parseContract(core.reviewed_audio_observation(encodeContract(state), encodeContract(observation), passID, confirmedEditionID, now));
}

import type {CatalogCandidate, RecommendationInput, Recommendation, SyncBatch, ApprovedSyncPacket, SealedSyncBatch, OpenedSyncBatch} from "./portable-services.js";
export interface RecommendationCore {
  recommendations(input: string): string;
  social_recommendations(state: string, candidates: string, excluded: string, policy: string, now: bigint): string;
}
/** Supply only independently verified shared ratings, or use audited socialRecommendations. */
export function recommendations(core: RecommendationCore, input: RecommendationInput): Recommendation[] {
  return parseContract(core.recommendations(encodeContract(input)));
}
/** Rebuilds signed history and rejects forged projections or revoked issuer pins. */
export function socialRecommendations(core: RecommendationCore, state: SocialState, candidates: CatalogCandidate[], excluded: string[], policy: AuthorityPolicy, now: bigint): Recommendation[] {
  checkTime(now);
  return parseContract(core.social_recommendations(encodeContract(state), encodeContract(candidates), encodeContract(excluded), encodeContract(policy), now));
}
export interface EncryptedSyncCore {
  prepare_sync_batch(actor: string, revision: bigint, recipients: string, plaintext: Uint8Array): string;
  seal_sync_batch(batch: string, plaintext: Uint8Array, key: Uint8Array, request: string, presentation: string, policy: string, now: bigint): string;
  open_sync_batch(packet: string, key: Uint8Array, recipient: string, policy: string, now: bigint): string;
}
function checkPairingKey(key: Uint8Array): void {
  if (!(key instanceof Uint8Array) || key.length !== 32) throw new Error("Pairing key must contain exactly 32 bytes");
}
/** Preparation computes the digest of these exact bytes. It does not authorize or send them. */
export function prepareSyncBatch(core: EncryptedSyncCore, actor: string, revision: bigint, recipients: string[], plaintext: Uint8Array): SyncBatch {
  checkTime(revision);
  return parseContract(core.prepare_sync_batch(actor, revision, encodeContract(recipients), plaintext));
}
/** Obtain fresh explicit host consent first. Atomically consume replay_key and persist the
 * returned packet before transport. Retries send the stored packet, never sign/seal again. */
export function sealSyncBatch(core: EncryptedSyncCore, batch: SyncBatch, plaintext: Uint8Array, key: Uint8Array, request: string, presentation: string, policy: AuthorityPolicy, now: bigint): SealedSyncBatch {
  checkTime(now); checkPairingKey(key);
  return parseContract(core.seal_sync_batch(encodeContract(batch), plaintext, key, request, presentation, encodeContract(policy), now));
}
/** Authenticate before decoding payload. This does not apply content, establish peer trust,
 * or consume a replay. Commit batch ID/digest with the checked payload atomically. */
export function openSyncBatch(core: EncryptedSyncCore, packet: ApprovedSyncPacket, key: Uint8Array, recipient: string, policy: AuthorityPolicy, now: bigint): OpenedSyncBatch {
  checkTime(now); checkPairingKey(key);
  const opened = parseContract<Omit<OpenedSyncBatch, "plaintext"> & {plaintext: number[]}>(core.open_sync_batch(encodeContract(packet), key, recipient, encodeContract(policy), now));
  return {...opened, plaintext: Uint8Array.from(opened.plaintext)};
}
