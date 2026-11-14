// SPDX-License-Identifier: MIT OR Apache-2.0
import * as core from "../sdk/typescript/pkg/nodejs/booknook_wasm.js";
import {BookNookFeedSourcesV1, type FeedSourceCoreV1, type FeedSourceProviderV1, type FeedCacheStoreV1} from "../sdk/typescript/index.js";
const generated: FeedSourceCoreV1 = core;
export async function actualGeneratedSource(raw: Uint8Array, provider: FeedSourceProviderV1, store: FeedCacheStoreV1, id: string): Promise<void> {
  const runtime = new BookNookFeedSourcesV1(generated, raw, provider);
  try {runtime.activate(id); const read = await runtime.fetch(id); try {await read.store(store);} finally {read.close();}}
  finally {runtime.close();}
}
