// SPDX-License-Identifier: MIT OR Apache-2.0
import * as core from "../sdk/typescript/pkg/nodejs/booknook_wasm.js";
import * as webCore from "../sdk/typescript/pkg/web/booknook_wasm.js";
import {BookNookFeedExportReviewV1, BookNookFeedImportReviewV1, BookNookFeedProjectionReviewV1,
  validateFeedSectionV1, validateFeedConfigurationV1, emptyFeedConfigurationV1,
  type FeedSdkCoreV1, type FeedExportSdkCoreV1, type FeedConfigurationV1, type FeedSectionV1} from "../sdk/typescript/index.js";
const implementedByGeneratedModule: FeedSdkCoreV1 = core;
// Existing adapters implement only the issued members. Export is independently additive.
const oldAdapterShape: FeedSdkCoreV1 = {
  validate_feed_section_v1: core.validate_feed_section_v1,
  validate_feed_configuration_v1: core.validate_feed_configuration_v1,
  empty_feed_configuration_v1: core.empty_feed_configuration_v1,
  FeedImportReviewV1: core.FeedImportReviewV1,
  FeedProjectionReviewV1: core.FeedProjectionReviewV1,
};
const exportNode: FeedExportSdkCoreV1 = core;
const exportWeb: FeedExportSdkCoreV1 = webCore;
export function generatedExportTypes(current: Uint8Array, selection: Uint8Array): FeedSectionV1 {
  validateFeedConfigurationV1(oldAdapterShape, current);
  const review = BookNookFeedExportReviewV1.prepare(exportNode, current, selection);
  const webReview = BookNookFeedExportReviewV1.prepare(exportWeb, current, selection);
  try { webReview.preview(); return review.confirm(current); }
  finally { review.close(); webReview.close(); }
}
export function generatedFeedTypes(raw: Uint8Array, current: Uint8Array,
  selection: Uint8Array, resolutions: Uint8Array): FeedConfigurationV1 {
  validateFeedSectionV1(implementedByGeneratedModule, raw);
  validateFeedConfigurationV1(core, current);
  emptyFeedConfigurationV1(core);
  const review = BookNookFeedImportReviewV1.prepare(core, current, raw);
  const resolved = review.resolve(current, raw, resolutions);
  try {
    const candidate = resolved.confirm(current, raw);
    const selected = BookNookFeedProjectionReviewV1.prepare(core, current, selection);
    try { selected.confirm(current, selected.preview().origin); } finally { selected.close(); }
    return candidate;
  } finally { resolved.close(); review.close(); }
}
