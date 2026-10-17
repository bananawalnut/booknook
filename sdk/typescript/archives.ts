// SPDX-License-Identifier: MIT OR Apache-2.0
import type {PortableArchive} from "./archive-types.js";
import {encodeContract, parseContract} from "./contract-json.js";
export type {PortableArchive, SocialArchiveBundle, ArchiveBinding, BindingHistory,
  PendingEnvelope, BindingDocument, KeyBinding} from "./archive-types.js";
export interface PortableArchiveCore {
  validate_portable_archive(rawJson: string): string;
}
/** Original JSON is essential: parsing first would conceal duplicate keys or round u64s.
 * This checks the whole envelope, asset checksums and pending relay signatures; it does
 * not trust imported identities, persist data, or validate EPUB/PDF format internals. */
export function validatePortableArchive(core: PortableArchiveCore, rawJson: string): PortableArchive {
  return parseContract(core.validate_portable_archive(rawJson));
}
/** Use raw roundtrip for exact u64 values outside JavaScript's safe integer range.
 * Outer JSON whitespace/order may change. Embedded signed strings and assets do not. */
export function roundtripPortableArchive(core: PortableArchiveCore, rawJson: string): string {
  return core.validate_portable_archive(rawJson);
}
/** Validate a host-built typed envelope before export. Never drops its social bundle. */
export function encodePortableArchive(core: PortableArchiveCore, archive: PortableArchive): string {
  return core.validate_portable_archive(encodeContract(archive));
}
