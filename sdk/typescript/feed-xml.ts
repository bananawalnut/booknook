// SPDX-License-Identifier: MIT OR Apache-2.0
import {parseContract} from "./contract-json.js";
export interface FeedNormalizationContextV1 {
  schema: "booknook.feed-normalization-context.v1"; source_id: string; provider_id: string;
  requested_url: string; final_url: string; expected_format: "auto" | "rss" | "atom";
}
/** Strings are inert text. literal-markup is a literal fallback, not HTML extraction. */
export interface InertFeedTextV1 {representation: "text" | "literal-markup" | "xhtml-text"; text: string}
export interface NormalizedFeedDateV1 {
  status: "missing" | "invalid" | "parsed"; raw: string | null;
  /** Non-leap Unix seconds (possibly negative). Leap seconds retain the preceding
   * whole Unix second and nanoseconds in [1e9,2e9), matching Chrono; raw is exact. */
  unix_seconds: number | null; nanoseconds: number | null;
}
export interface NormalizedFeedEntryV1 {
  identity: {kind: "native" | "derived-sha256"; value: string};
  title: InertFeedTextV1 | null; authors: string[]; summary: InertFeedTextV1 | null;
  content: InertFeedTextV1 | null; omitted_content: boolean;
  links: {url: string; rel: "alternate" | "related"}[]; omitted_links: number;
  published: NormalizedFeedDateV1; updated: NormalizedFeedDateV1;
  first_source_ordinal: number; duplicate_count: number; conflicting_identity: boolean;
}
/** Produced-only data. This type does not validate arbitrary reopened cache JSON. */
export interface NormalizedFeedV1 {
  schema: "booknook.normalized-feed.v1"; normalizer_profile: "booknook.rss2-atom.inert.v1";
  format: "rss" | "atom";
  provenance: {source_id: string; provider_id: string; requested_url: string; final_url: string; raw_sha256: string};
  title: InertFeedTextV1 | null; entries: NormalizedFeedEntryV1[];
}
export interface FeedXmlCoreV1 { normalize_feed_xml_v1(contextBytes: Uint8Array, xmlBytes: Uint8Array): string }
/** Pure original-byte normalization. Render strings as text only; links require a
 * separate explicit host action. No fetch, cache, activation or renderer runs. */
export function normalizeFeedXmlV1(core: FeedXmlCoreV1, contextBytes: Uint8Array, xmlBytes: Uint8Array): NormalizedFeedV1 {
  return parseContract(core.normalize_feed_xml_v1(contextBytes,xmlBytes));
}
