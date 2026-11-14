// SPDX-License-Identifier: MIT OR Apache-2.0
import * as core from "../sdk/typescript/pkg/nodejs/booknook_wasm.js";
import {BookNookFeedImportReviewV1, BookNookFeedProjectionReviewV1,
  validateFeedSectionV1, validateFeedConfigurationV1, emptyFeedConfigurationV1,
  type FeedSdkCoreV1, type FeedConfigurationV1} from "../sdk/typescript/index.js";
const implementedByGeneratedModule: FeedSdkCoreV1 = core;
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
