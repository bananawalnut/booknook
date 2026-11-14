// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif

/// Data-only feed SDK. Preserve source bytes through validation/review; no
/// persistence, fetch, activation, identity or operational authority is implied.
public enum BookNookFeedsV1 {
    public static func validateSection(_ bytes: Data) throws -> String {
        try validateFeedSectionV1(raw: bytes)
    }
    public static func validateConfiguration(_ bytes: Data) throws -> String {
        try validateFeedConfigurationV1(raw: bytes)
    }
    public static func emptyConfiguration() throws -> String {
        try emptyFeedConfigurationV1()
    }
    public static func reviewImport(configuration: Data, incoming: Data?) throws -> BookNookFeedImportReviewV1 {
        BookNookFeedImportReviewV1(try FeedImportReviewV1(configurationBytes: configuration, incomingBytes: incoming))
    }
    public static func reviewProjection(configuration: Data, selection: Data) throws -> BookNookFeedProjectionReviewV1 {
        BookNookFeedProjectionReviewV1(try FeedProjectionReviewV1(configurationBytes: configuration, selectionBytes: selection))
    }
}
private enum FeedReviewError: LocalizedError {
    case closed
    var errorDescription: String? { "Feed review is closed." }
}
/// Owns the actual UniFFI review object; serialized previews cannot create one.
/// Hosts own consent and atomic persistence of a returned candidate.
public final class BookNookFeedImportReviewV1 {
    private let lock = NSLock()
    private var handle: FeedImportReviewV1?
    fileprivate init(_ handle: FeedImportReviewV1) { self.handle = handle }
    private func live<T>(_ operation: (FeedImportReviewV1) throws -> T) throws -> T {
        lock.lock(); defer { lock.unlock() }
        guard let handle else { throw FeedReviewError.closed }
        return try operation(handle)
    }
    public func preview() throws -> String { try live { try $0.preview() } }
    /// One complete choice batch bound to the original exact bytes/configuration.
    public func resolve(configuration: Data, incoming: Data?, resolutions: Data) throws -> BookNookFeedImportReviewV1 {
        try live { BookNookFeedImportReviewV1(try $0.resolve(configurationBytes: configuration, incomingBytes: incoming, resolutionBytes: resolutions)) }
    }
    public func confirm(configuration: Data, incoming: Data?) throws -> String {
        try live { try $0.confirm(configurationBytes: configuration, incomingBytes: incoming) }
    }
    public func close() { lock.lock(); defer { lock.unlock() }; handle = nil }
}
/// Exact destination and current configuration remain bound to the Rust review.
/// Returning a projection does not authorize disclosure or network access.
public final class BookNookFeedProjectionReviewV1 {
    private let lock = NSLock()
    private var handle: FeedProjectionReviewV1?
    fileprivate init(_ handle: FeedProjectionReviewV1) { self.handle = handle }
    private func live<T>(_ operation: (FeedProjectionReviewV1) throws -> T) throws -> T {
        lock.lock(); defer { lock.unlock() }
        guard let handle else { throw FeedReviewError.closed }
        return try operation(handle)
    }
    public func preview() throws -> String { try live { try $0.preview() } }
    public func confirm(configuration: Data, origin: String) throws -> String {
        try live { try $0.confirm(configurationBytes: configuration, origin: origin) }
    }
    public func close() { lock.lock(); defer { lock.unlock() }; handle = nil }
}
