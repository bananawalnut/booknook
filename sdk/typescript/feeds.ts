// SPDX-License-Identifier: MIT OR Apache-2.0
import {parseContract} from "./contract-json.js";

export interface FeedCollectionV1 { id: string; title: string }
export interface FeedSubscriptionV1 {
  id: string; url: string; format: "auto" | "rss" | "atom"; title: string;
  enabled: boolean; collectionIds: string[]; disclosure: "private" | "selected";
}
export interface FeedSectionV1 {
  schema: "castalia.feeds-section.v1"; version: 1;
  subscriptions: FeedSubscriptionV1[]; collections: FeedCollectionV1[];
  preferences: { order: "newest-first" };
}
export interface FeedConfigurationV1 {
  schema: "booknook.feed-configuration.v1"; revision: number; feeds: FeedSectionV1;
}
export type FeedConflictV1 =
  | {target: "collection"; id: string; current: FeedCollectionV1; incoming: FeedCollectionV1}
  | {target: "subscription"; id: string; current: FeedSubscriptionV1; incoming: FeedSubscriptionV1};
export interface FeedConflictResolutionV1 {
  target: "collection" | "subscription"; id: string; choice: "current" | "incoming";
}
export interface FeedImportPreviewV1 {
  schema: "booknook.feed-import-review.v1";
  conflicts: FeedConflictV1[]; candidate: FeedConfigurationV1 | null;
}
export interface FeedDisclosureSelectionV1 {
  schema: "booknook.feed-disclosure-selection.v1"; origin: string;
  subscriptionIds: string[]; collectionIds: string[];
  memberships: {subscriptionId: string; collectionIds: string[]}[];
}
export interface SelectedFeedProjectionV1 {
  schema: "booknook.selected-feeds.v1"; origin: string; feeds: FeedSectionV1;
}
export interface FeedImportReviewHandleV1 {
  preview(): string;
  resolve(configurationBytes: Uint8Array, incomingBytes: Uint8Array | null | undefined,
    resolutionBytes: Uint8Array): FeedImportReviewHandleV1;
  confirm(configurationBytes: Uint8Array, incomingBytes: Uint8Array | null | undefined): string;
  free(): void;
}
export interface FeedProjectionReviewHandleV1 {
  preview(): string;
  confirm(configurationBytes: Uint8Array, origin: string): string;
  free(): void;
}
/** The real generated module implements this independent versioned domain.
 * Original bytes enter Rust unchanged; these APIs never fetch, store or activate. */
export interface FeedSdkCoreV1 {
  validate_feed_section_v1(raw: Uint8Array): string;
  validate_feed_configuration_v1(raw: Uint8Array): string;
  empty_feed_configuration_v1(): string;
  FeedImportReviewV1: new(configurationBytes: Uint8Array,
    incomingBytes?: Uint8Array | null) => FeedImportReviewHandleV1;
  FeedProjectionReviewV1: new(configurationBytes: Uint8Array,
    selectionBytes: Uint8Array) => FeedProjectionReviewHandleV1;
}
export function validateFeedSectionV1(core: FeedSdkCoreV1, raw: Uint8Array): FeedSectionV1 {
  return parseContract(core.validate_feed_section_v1(raw));
}
export function validateFeedConfigurationV1(core: FeedSdkCoreV1, raw: Uint8Array): FeedConfigurationV1 {
  return parseContract(core.validate_feed_configuration_v1(raw));
}
export function emptyFeedConfigurationV1(core: FeedSdkCoreV1): FeedConfigurationV1 {
  return parseContract(core.empty_feed_configuration_v1());
}
/** Opaque in-memory review; JSON previews cannot reconstruct it. The host owns
 * consent and atomic persistence. Confirmation returns data only, never permission. */
export class BookNookFeedImportReviewV1 {
  private constructor(private handle: FeedImportReviewHandleV1 | undefined) {}
  static prepare(core: FeedSdkCoreV1, current: Uint8Array,
    incoming?: Uint8Array | null): BookNookFeedImportReviewV1 {
    return new BookNookFeedImportReviewV1(new core.FeedImportReviewV1(current, incoming));
  }
  private live(): FeedImportReviewHandleV1 {
    if (!this.handle) throw new Error("Feed review is closed");
    return this.handle;
  }
  preview(): FeedImportPreviewV1 { return parseContract(this.live().preview()); }
  /** One complete choice batch for the original exact input and configuration. */
  resolve(current: Uint8Array, incoming: Uint8Array | null | undefined,
    resolutionBytes: Uint8Array): BookNookFeedImportReviewV1 {
    return new BookNookFeedImportReviewV1(this.live().resolve(current, incoming, resolutionBytes));
  }
  confirm(current: Uint8Array, incoming?: Uint8Array | null): FeedConfigurationV1 {
    return parseContract(this.live().confirm(current, incoming));
  }
  close(): void { const handle = this.handle; this.handle = undefined; handle?.free(); }
}
/** Destination and unchanged configuration are rechecked before returning the
 * selected projection. Unreferenced collection labels/IDs are never included. */
export class BookNookFeedProjectionReviewV1 {
  private constructor(private handle: FeedProjectionReviewHandleV1 | undefined) {}
  static prepare(core: FeedSdkCoreV1, current: Uint8Array,
    selectionBytes: Uint8Array): BookNookFeedProjectionReviewV1 {
    return new BookNookFeedProjectionReviewV1(new core.FeedProjectionReviewV1(current, selectionBytes));
  }
  private live(): FeedProjectionReviewHandleV1 {
    if (!this.handle) throw new Error("Feed projection review is closed");
    return this.handle;
  }
  preview(): SelectedFeedProjectionV1 { return parseContract(this.live().preview()); }
  confirm(current: Uint8Array, origin: string): SelectedFeedProjectionV1 {
    return parseContract(this.live().confirm(current, origin));
  }
  close(): void { const handle = this.handle; this.handle = undefined; handle?.free(); }
}
