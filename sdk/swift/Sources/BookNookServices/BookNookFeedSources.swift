// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
#if canImport(BookNookNative)
import BookNookNative
#endif
/// Executable host dependency; descriptor metadata does not grant network access.
/// Implementations own bounded safe I/O and observe the explicit cancellation token.
public protocol BookNookFeedSourceProviderV1: Sendable {
    var descriptorJSON: String { get }
    func fetch(requestJSON: String, cancellation: FeedSourceCancellationV1) async throws -> BookNookFeedSourceReplyV1
}
public struct BookNookFeedSourceReplyV1: Sendable {
    public let responseBytes: Data
    public let untrustedBody: Data
    public init(responseBytes: Data, untrustedBody: Data) {self.responseBytes = responseBytes; self.untrustedBody = untrustedBody}
}
/// Async staging followed by a synchronous atomic durable CAS. Async final
/// transactions need a separately qualified contract; a snapshot is not a permit.
public struct BookNookFeedCacheStageV1: Sendable {
    public let reference: String
    public let existingBody: Data?
    public init(reference: String, existingBody: Data? = nil) {self.reference = reference; self.existingBody = existingBody}
}
public protocol BookNookFeedCacheStoreV1: Sendable {
    func stage(conditionJSON: String, untrustedBody: Data, cancellation: FeedSourceCancellationV1) async throws -> BookNookFeedCacheStageV1
    func compareAndStore(staged: String, conditionJSON: String, untrustedBody: Data) throws
}
private final class SourceBridge: FeedSourceHostV1, @unchecked Sendable {
    let provider: any BookNookFeedSourceProviderV1
    let descriptor: String
    init(_ provider: any BookNookFeedSourceProviderV1) {self.provider = provider; self.descriptor = provider.descriptorJSON}
    func descriptorJson() throws -> String {descriptor}
    func fetch(requestJson: String, cancellation: FeedSourceCancellationV1) async throws -> FeedSourceReplyV1 {
        do {let result = try await provider.fetch(requestJSON: requestJson, cancellation: cancellation)
            return FeedSourceReplyV1(responseBytes: result.responseBytes, body: result.untrustedBody)
        } catch {throw NativeError.Failure(message: error.localizedDescription)}
    }
}
private final class CacheBridge: FeedCacheHostV1, @unchecked Sendable {
    let store: any BookNookFeedCacheStoreV1
    init(_ store: any BookNookFeedCacheStoreV1) {self.store = store}
    func stage(conditionJson: String, body: Data, cancellation: FeedSourceCancellationV1) async throws -> FeedCacheStageV1 {
        do {let staged = try await store.stage(conditionJSON: conditionJson, untrustedBody: body, cancellation: cancellation); return FeedCacheStageV1(reference: staged.reference, existingBody: staged.existingBody)}
        catch {throw NativeError.Failure(message: error.localizedDescription)}
    }
    func compareAndStore(staged: String, conditionJson: String, body: Data) throws {
        do {try store.compareAndStore(staged: staged, conditionJSON: conditionJson, untrustedBody: body)}
        catch {throw NativeError.Failure(message: error.localizedDescription)}
    }
}
/// Independent host-local source runtime. Import/config/provider replacement
/// revokes all activations and pending reads. No Desk or vault authority involved.
public final class BookNookFeedSourcesV1: @unchecked Sendable {
    private let handle: FeedSourceSessionV1
    private init(handle: FeedSourceSessionV1) { self.handle = handle }
    /// Explicit isolated native HTTPS transport. Offline construction; sources
    /// remain inactive until the host calls activate. Returns untrusted XML bytes.
    public static func publicHTTPS(configuration: Data) throws -> BookNookFeedSourcesV1 {
        BookNookFeedSourcesV1(handle: try FeedSourceSessionV1.publicHttps(configurationBytes: configuration))
    }
    public init(configuration: Data, provider: any BookNookFeedSourceProviderV1) throws {
        handle = try FeedSourceSessionV1(configurationBytes: configuration, host: SourceBridge(provider))
    }
    public func replaceConfiguration(_ raw: Data) throws {try handle.replaceConfiguration(configurationBytes: raw)}
    public func replaceProvider(_ provider: any BookNookFeedSourceProviderV1) throws {try handle.replaceProvider(host: SourceBridge(provider))}
    public func activate(sourceID: String) throws {try handle.activate(sourceId: sourceID)}
    public func disable(sourceID: String) throws {try handle.disable(sourceId: sourceID)}
    public func close() throws {try handle.close()}
    public func fetch(sourceID: String, conditional: Data? = nil) async throws -> BookNookFeedReadV1 {
        try Task.checkCancellation()
        let attempt = try handle.begin(sourceId: sourceID, conditionalBytes: conditional)
        return try await withTaskCancellationHandler {
            do {
                let read = try await attempt.execute()
                try Task.checkCancellation()
                return BookNookFeedReadV1(read)
            } catch {try Task.checkCancellation(); throw error}
        } onCancel: {_ = attempt.cancel()}
    }
}
/// Opaque read backed by a real Rust object. Bytes are untrusted; no parsing or
/// rendering approval is inferred. Closing cancels later admission to storage.
public final class BookNookFeedReadV1: @unchecked Sendable {
    private let handle: FeedSourceReadV1
    fileprivate init(_ handle: FeedSourceReadV1) {self.handle = handle}
    public func requestJSON() throws -> String {try handle.request()}
    public func responseJSON() throws -> String {try handle.response()}
    public func untrustedBody() -> Data {handle.body()}
    public func cacheConditionJSON() throws -> String {try handle.cacheCondition()}
    @discardableResult public func cancel() -> Bool {handle.cancel()}
    public func close() {_ = handle.cancel()}
    public func store(_ store: any BookNookFeedCacheStoreV1) async throws {
        try Task.checkCancellation()
        try await withTaskCancellationHandler {
            do {try await handle.store(host: CacheBridge(store)); try Task.checkCancellation()}
            catch {try Task.checkCancellation(); throw error}
        } onCancel: {_ = self.handle.cancel()}
    }
}
