// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif
/// Pure original-byte RSS2/Atom normalization. The returned JSON is produced-only
/// inert data, not a decoder for reopened durable caches. Render all strings as
/// text; links require a separate explicit host action. No network/cache runs.
public enum BookNookFeedXMLV1 {
    public static func normalize(context: Data, xml: Data) throws -> String {
        try normalizeFeedXmlV1(contextBytes: context, xmlBytes: xml)
    }
}
