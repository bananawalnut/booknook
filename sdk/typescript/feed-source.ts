// SPDX-License-Identifier: MIT OR Apache-2.0
import {parseContract} from "./contract-json.js";
export interface FeedSourceDescriptorV1 {
  schema: "booknook.feed-source-provider.v1"; version: 1; id: string; available: boolean;
  protocols: ["public-xml-bytes.v1"];
}
export interface FeedValidatorsV1 {etag: string | null; last_modified: string | null}
export interface FeedConditionalHintV1 {
  schema: "booknook.feed-conditional-hint.v1"; source_id: string; provider_id: string;
  url: string; body_sha256: string; validators: FeedValidatorsV1;
}
export interface FeedSourceRequestV1 {
  schema: "booknook.feed-source-request.v1"; runtime_id: string; request_id: string;
  source_id: string; provider_id: string; configuration_generation: number;
  provider_generation: number; activation_generation: number;
  url: string; format: "auto" | "rss" | "atom"; protocol: "public-xml-bytes.v1";
  conditional: FeedConditionalHintV1 | null;
}
export interface FeedSourceResponseV1 {
  schema: "booknook.feed-source-response.v1"; runtime_id: string; request_id: string;
  source_id: string; provider_id: string; configuration_generation: number;
  provider_generation: number; activation_generation: number;
  final_url: string; content_type: string | null; body_sha256: string;
  outcome: "body" | "not-modified"; validators: FeedValidatorsV1;
}
export interface FeedCacheConditionV1 {
  schema: "booknook.feed-cache-condition.v1"; runtime_id: string; request_id: string;
  source_id: string; provider_id: string; configuration_generation: number;
  provider_generation: number; activation_generation: number; body_sha256: string;
  url: string; final_url: string; outcome: "body" | "not-modified";
  expected_body_sha256: string | null; validators: FeedValidatorsV1;
}
export interface FeedSourceReplyV1 {responseBytes: Uint8Array; untrustedBody: Uint8Array}
export interface FeedCancellationV1 {isCancelled(): boolean}
/** Explicit executable host dependency. Descriptor metadata installs/grants nothing.
 * The host owns bounded credential-free safe I/O; the SDK does not certify it. */
export interface FeedSourceProviderV1 {
  readonly descriptorBytes: Uint8Array;
  fetch(request: Readonly<FeedSourceRequestV1>, cancellation: FeedCancellationV1): Promise<FeedSourceReplyV1>;
}
export interface FeedCacheCommitterV1 {
  /** Atomically compare the durable runtime/generation/source key and complete
   * its durable write before returning. Never reenter the source runtime. */
  compare_and_store(conditionJson: string, untrustedBody: Uint8Array): undefined;
}
export interface FeedCacheStageV1 {committer: FeedCacheCommitterV1; existingBody?: Uint8Array | null}
export interface FeedCacheStoreV1 {
  /** Async staging only. An async final transaction, including an unqualified
   * IndexedDB transaction, cannot implement this v1 synchronous final boundary. */
  stage(condition: Readonly<FeedCacheConditionV1>, untrustedBody: Uint8Array,
    cancellation: FeedCancellationV1): Promise<FeedCacheStageV1>;
}
export interface FeedSourceAttemptHandleV1 {
  dispatch(): string; cancel(): boolean; is_cancelled(): boolean;
  complete(responseBytes: Uint8Array, body: Uint8Array): FeedSourceReadHandleV1; free(): void;
}
export interface FeedSourceReadHandleV1 {
  request(): string; response(): string; body(): Uint8Array; cache_condition(): string;
  cancel(): boolean; commit_cache(committer: FeedCacheCommitterV1, existingBody?: Uint8Array | null): void; free(): void;
}
export interface FeedSourceSessionHandleV1 {
  replace_configuration(raw: Uint8Array): void; replace_provider(raw: Uint8Array): void;
  activate(sourceId: string): void; disable(sourceId: string): void;
  begin(sourceId: string, conditionalBytes?: Uint8Array | null): FeedSourceAttemptHandleV1; close(): void; free(): void;
}
export interface FeedSourceCoreV1 {
  validate_feed_source_descriptor_v1(raw: Uint8Array): string;
  FeedSourceSessionV1: new(configurationBytes: Uint8Array, descriptorBytes: Uint8Array) => FeedSourceSessionHandleV1;
}
export function validateFeedSourceDescriptorV1(core: FeedSourceCoreV1, raw: Uint8Array): FeedSourceDescriptorV1 {
  return parseContract(core.validate_feed_source_descriptor_v1(raw));
}
/** Local runtime only; restoring portable configuration always revokes activation. */
const readKey = Symbol("actual-feed-read");
export class BookNookFeedSourcesV1 {
  private handle: FeedSourceSessionHandleV1 | undefined;
  private provider: FeedSourceProviderV1;
  private readonly pending = new Set<FeedSourceAttemptHandleV1>();
  constructor(core: FeedSourceCoreV1, configuration: Uint8Array, provider: FeedSourceProviderV1) {
    this.handle = new core.FeedSourceSessionV1(configuration, provider.descriptorBytes);
    this.provider = provider;
  }
  private live(): FeedSourceSessionHandleV1 {if (!this.handle) throw new Error("Feed sources are closed"); return this.handle;}
  replaceConfiguration(raw: Uint8Array): void {this.live().replace_configuration(raw);}
  replaceProvider(provider: FeedSourceProviderV1): void {
    this.live().replace_provider(provider.descriptorBytes); this.provider = provider;
  }
  activate(sourceId: string): void {this.live().activate(sourceId);}
  disable(sourceId: string): void {this.live().disable(sourceId);}
  async fetch(sourceId: string, signal?: AbortSignal, conditionalBytes?: Uint8Array): Promise<BookNookFeedReadV1> {
    if (signal?.aborted) throw new Error("Feed request cancelled");
    const attempt = this.live().begin(sourceId, conditionalBytes), provider = this.provider;
    let settled = false;
    this.pending.add(attempt);
    const cancel = () => {attempt.cancel();}; signal?.addEventListener("abort", cancel, {once: true});
    try {
      const request: FeedSourceRequestV1 = parseContract(attempt.dispatch());
      const reply = await provider.fetch(Object.freeze(request), {isCancelled: () => settled || attempt.is_cancelled()});
      if (signal?.aborted) attempt.cancel();
      return new BookNookFeedReadV1(attempt.complete(reply.responseBytes, reply.untrustedBody), readKey);
    } finally {settled = true; signal?.removeEventListener("abort", cancel); this.pending.delete(attempt); attempt.free();}
  }
  close(): void {const handle = this.handle; if (!handle) return; handle.close(); this.handle = undefined; for (const pending of this.pending) pending.cancel(); handle.free();}
}
/** Opaque completed read. Bytes remain untrusted, with no XML/rendering approval. */
export class BookNookFeedReadV1 {
  private handle: FeedSourceReadHandleV1 | undefined;
  constructor(handle: FeedSourceReadHandleV1, key: typeof readKey) {if (key !== readKey) throw new Error("An actual source attempt is required"); this.handle = handle;}
  private live(): FeedSourceReadHandleV1 {if (!this.handle) throw new Error("Feed read is closed"); return this.handle;}
  request(): FeedSourceRequestV1 {return parseContract(this.live().request());}
  response(): FeedSourceResponseV1 {return parseContract(this.live().response());}
  untrustedBody(): Uint8Array {return this.live().body();}
  cacheCondition(): FeedCacheConditionV1 {return parseContract(this.live().cache_condition());}
  cancel(): boolean {return this.live().cancel();}
  async store(store: FeedCacheStoreV1, signal?: AbortSignal): Promise<void> {
    if (signal?.aborted) {this.cancel(); throw new Error("Feed cache operation cancelled");}
    const cancel = () => {this.handle?.cancel();}; signal?.addEventListener("abort", cancel, {once: true});
    try {
      const staged = await store.stage(Object.freeze(this.cacheCondition()), this.untrustedBody(),
        {isCancelled: () => {try {this.cacheCondition(); return false;} catch {return true;}}});
      // A condition snapshot before the await is never a commit permit.
      this.cacheCondition(); this.live().commit_cache(staged.committer, staged.existingBody);
    } finally {signal?.removeEventListener("abort", cancel);}
  }
  close(): void {const handle = this.handle; this.handle = undefined; handle?.cancel(); handle?.free();}
}
