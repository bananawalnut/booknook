// SPDX-License-Identifier: MIT OR Apache-2.0
import type {State} from "./types.js";
import type {NotebookStorage, WasmCore} from "./index.js";
import type {RecommendationInput, Recommendation} from "./portable-services.js";
import type {ProviderRegistry, ProviderOperation, ProviderExecution, RuntimeProviderDescriptor, SourcePosition} from "./provider-types.js";
import {checkTime, encodeContract, parseContract} from "./contract-json.js";
export type {ProviderRegistry, RuntimeProviderDescriptor, RuntimeCapability, ProviderOperation,
  ProviderRequest, ProviderResponse, ProviderExecution, ProviderResult, CatalogInput, SourceCatalogEntry, SourcePosition} from "./provider-types.js";
export type * as Providers from "./provider-types.js";

/** Host-supplied executable adapter. A downloaded descriptor cannot register code.
 * Keep descriptorJSON immutable; replace the registration to change its source. */
export interface BookNookProvider {
  readonly descriptorJSON: string;
  execute(requestJSON: string, signal?: AbortSignal): Promise<ProviderReply>;
}
export interface ProviderReply {
  /** Original response JSON; shared Rust validation runs before JavaScript decoding. */
  responseJSON: string;
  /** Content is separate from JSON to preserve the native 128 MiB content bound. */
  contentBytes?: Uint8Array;
}
export interface ProviderCore {
  provider_registry(registry: string): string;
  provider_prepare_operation(registry: string, generation: bigint, providerID: string, operation: string, requestID: string): string;
  provider_validate_reply(registry: string, generation: bigint, request: string, response: string, bytes: Uint8Array, now: bigint): string;
  provider_validate_execution(registry: string, generation: bigint, execution: string, bytes: Uint8Array, now: bigint): string;
  provider_apply_catalog(state: string, registry: string, generation: bigint, execution: string, now: bigint): string;
  provider_apply_source_position(state: string, registry: string, generation: bigint, execution: string,
    observation: string, passID: string, now: bigint): string;
}
export interface ValidatedProviderReply {
  executionJSON: string;
  execution: ProviderExecution;
  contentBytes: Uint8Array;
}
export interface ProviderServiceOptions {
  /** UTC seconds. The shared verifier remains authoritative for range and freshness. */
  now?: () => bigint;
  requestID?: () => string;
}
interface Registration { host: BookNookProvider; rawDescriptor: string }
function cancelled(signal?: AbortSignal): void {
  if (signal?.aborted) throw new DOMException("The provider operation was cancelled", "AbortError");
}
function requestID(): string {
  if (!globalThis.crypto?.randomUUID) throw new Error("This host must supply a unique requestID function");
  return globalThis.crypto.randomUUID();
}

/** Native-compatible provider validation and catalog reconciliation over host storage.
 * Network fetches never hold a storage lock. A fresh state is loaded before atomic CAS.
 * A completed CAS is reported even if cancellation arrives after the commit began. */
export class BookNookServices {
  private generation = 0n;
  private registryJSON = "";
  private registrations = new Map<string, Registration>();
  private activeCommits = 0;
  private readonly now: () => bigint;
  private readonly nextRequestID: () => string;

  private constructor(private readonly core: ProviderCore & WasmCore,
    private readonly storage: NotebookStorage, providers: readonly BookNookProvider[], options: ProviderServiceOptions) {
    this.now = options.now ?? (() => BigInt(Math.floor(Date.now() / 1000)));
    this.nextRequestID = options.requestID ?? requestID;
    this.install(providers);
  }
  static async open(core: ProviderCore & WasmCore, storage: NotebookStorage,
    providers: readonly BookNookProvider[], options: ProviderServiceOptions = {}): Promise<BookNookServices> {
    const services = new BookNookServices(core, storage, providers, options);
    await services.state();
    return services;
  }
  /** Replacement invalidates pending fetches. Refused while a local commit is in flight
   * so generation cannot change between its validation and the storage transaction. */
  replaceProviders(providers: readonly BookNookProvider[]): void {
    if (this.activeCommits) throw new Error("A local provider commit is in progress; retry registration afterward");
    if (this.generation >= BigInt(Number.MAX_SAFE_INTEGER)) throw new Error("Provider generation exceeds this JavaScript host's exact range");
    this.install(providers);
    this.generation += 1n;
  }
  descriptors(): RuntimeProviderDescriptor[] {
    return parseContract<ProviderRegistry>(this.registryJSON).providers;
  }
  private install(providers: readonly BookNookProvider[]): void {
    // Do not JSON.parse individual descriptors: that would discard duplicate keys.
    const raw = providers.map(provider => provider.descriptorJSON);
    const registryJSON = this.core.provider_registry('{"schema":"booknook.provider-registry.v1","providers":[' + raw.join(',') + ']}');
    const registry = parseContract<ProviderRegistry>(registryJSON);
    const registrations = new Map<string, Registration>();
    registry.providers.forEach((descriptor, index) => registrations.set(descriptor.id, {host: providers[index], rawDescriptor: raw[index]}));
    this.registryJSON = registryJSON;
    this.registrations = registrations;
  }
  private current(generation: bigint): void {
    if (this.generation !== generation) throw new Error("Provider registration changed; review the source and retry");
    for (const registration of this.registrations.values()) {
      if (registration.host.descriptorJSON !== registration.rawDescriptor) throw new Error("Provider descriptor changed without replacing its registration");
    }
  }
  private time(): bigint { const value = this.now(); checkTime(value); return value; }
  private async state(): Promise<State> {
    let state = await this.storage.load();
    if (!state) {
      const initial = parseContract<State>(this.core.new_state());
      state = await this.storage.compareAndSave(null, initial) ? initial : await this.storage.load();
    }
    if (!state) throw new Error("Notebook storage unavailable");
    this.core.validate_archive(encodeContract(state));
    return state;
  }
  /** Transport results remain descriptive data. Social exchange still requires the
   * caller's Nostr signature/binding, permission, membership and replay validation. */
  async fetch(providerID: string, operation: ProviderOperation, signal?: AbortSignal): Promise<ValidatedProviderReply> {
    cancelled(signal);
    const generation = this.generation;
    this.current(generation);
    const request = this.core.provider_prepare_operation(this.registryJSON, generation, providerID,
      encodeContract(operation), this.nextRequestID());
    const registration = this.registrations.get(providerID);
    if (!registration) throw new Error("Provider is not registered");
    const reply = await registration.host.execute(request, signal);
    cancelled(signal);
    this.current(generation);
    const bytes = reply.contentBytes ?? new Uint8Array();
    if (!(bytes instanceof Uint8Array) || bytes.byteLength > 128 * 1024 * 1024) throw new Error("Invalid or oversized provider content");
    // Snapshot provider-owned memory before hashing or exposing the validated result.
    const contentBytes = new Uint8Array(bytes);
    const executionJSON = this.core.provider_validate_reply(this.registryJSON, generation, request,
      reply.responseJSON, contentBytes, this.time());
    return {executionJSON, execution: parseContract(executionJSON), contentBytes};
  }
  async importCatalog(providerID: string, sourceID: string, signal?: AbortSignal): Promise<{
    state: State; result: {inserted: number; updated: number; unchanged: number}
  }> {
    const generation = this.generation;
    const fetched = await this.fetch(providerID, {kind:"catalog", source_id:sourceID}, signal);
    const state = await this.state();
    cancelled(signal); this.current(generation);
    const response = parseContract<{state: State; result: {inserted: number; updated: number; unchanged: number}}>(
      this.core.provider_apply_catalog(encodeContract(state), this.registryJSON, generation, fetched.executionJSON, this.time()));
    this.activeCommits += 1;
    try {
      if (!(await this.storage.compareAndSave(state.revision, response.state))) throw new Error("Storage conflict: reload before retrying the import");
      return response;
    } finally { this.activeCommits -= 1; }
  }
  /** Validates source/media/hash only. The host must parse and validate EPUB/PDF bytes
   * and commit assets atomically before representing content as locally accessible. */
  async acquireContent(providerID: string, sourceID: string, mediaType: "application/epub+zip" | "application/pdf",
    signal?: AbortSignal): Promise<ValidatedProviderReply> {
    return this.fetch(providerID, {kind:"content",source_id:sourceID,media_type:mediaType}, signal);
  }
  async sourcePositions(providerID: string, sourceIDs: string[], signal?: AbortSignal): Promise<ValidatedProviderReply> {
    return this.fetch(providerID, {kind:"source_positions",source_ids:sourceIDs}, signal);
  }
  /** Attach only after the reader selects the intended reading pass. The observation
   * retains its provider namespace and unmatched alignment; no percentage conversion. */
  async attachPosition(read: ValidatedProviderReply, observation: SourcePosition, passID: string,
    signal?: AbortSignal): Promise<{state: State; result: unknown}> {
    cancelled(signal);
    const generation = this.generation;
    this.current(generation);
    if (read.contentBytes.byteLength) throw new Error("A position response cannot contain book bytes");
    const state = await this.state();
    cancelled(signal); this.current(generation);
    const response = parseContract<{state: State; result: unknown}>(this.core.provider_apply_source_position(
      encodeContract(state), this.registryJSON, generation, read.executionJSON, encodeContract(observation), passID, this.time()));
    this.activeCommits += 1;
    try {
      if (!(await this.storage.compareAndSave(state.revision, response.state))) throw new Error("Storage conflict: reload before retrying the position");
      return response;
    } finally { this.activeCommits -= 1; }
  }
  async recommendations(providerID: string, input: RecommendationInput, signal?: AbortSignal): Promise<Recommendation[]> {
    const reply = await this.fetch(providerID, {kind:"recommendations",input}, signal);
    if (reply.execution.response.result.kind !== "recommendations") throw new Error("Unexpected provider result");
    return reply.execution.response.result.recommendations;
  }
}
