// SPDX-License-Identifier: MIT OR Apache-2.0
import * as core from "../sdk/typescript/pkg/nodejs/booknook_wasm.js";
import {normalizeFeedXmlV1, type FeedXmlCoreV1, type NormalizedFeedV1} from "../sdk/typescript/index.js";
const actualGenerated: FeedXmlCoreV1=core;
export function normalization(context: Uint8Array,xml: Uint8Array): NormalizedFeedV1 {
  return normalizeFeedXmlV1(actualGenerated,context,xml);
}
