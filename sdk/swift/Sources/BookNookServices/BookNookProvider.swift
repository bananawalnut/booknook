// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation

/// An executable host dependency, explicitly supplied by the embedding application.
/// A downloaded connection descriptor cannot instantiate or grant access to a provider.
///
/// Keep `descriptorJSON` immutable for the lifetime of this registration. Replacing a
/// source or its availability requires a new registration/generation. Requests and
/// responses use the versioned provider wire contracts; Rust validates original JSON
/// before reconciling data. Providers never receive an entire private notebook state.
public protocol BookNookProvider: Sendable {
    var descriptorJSON: String { get }
    func execute(requestJSON: String) async throws -> BookNookProviderResponse
}

public struct BookNookProviderResponse: Sendable {
    public let responseJSON: String
    /// Populated only for a content response. Catalog entries do not imply text access.
    public let contentBytes: Data
    public init(responseJSON: String, contentBytes: Data = Data()) {
        self.responseJSON = responseJSON
        self.contentBytes = contentBytes
    }
}

/// Closure adapter for an application's existing asynchronous source implementation.
/// The closure remains responsible for its own bounded I/O and cancellation handling.
public struct BookNookClosureProvider: BookNookProvider {
    public let descriptorJSON: String
    private let operation: @Sendable (String) async throws -> BookNookProviderResponse
    public init(descriptorJSON: String,
                execute: @escaping @Sendable (String) async throws -> BookNookProviderResponse) {
        self.descriptorJSON = descriptorJSON
        self.operation = execute
    }
    public func execute(requestJSON: String) async throws -> BookNookProviderResponse {
        try Task.checkCancellation()
        let response = try await operation(requestJSON)
        try Task.checkCancellation()
        return response
    }
}

public enum BookNookServiceError: LocalizedError, Sendable {
    case message(String)
    public var errorDescription: String? {
        switch self { case .message(let message): return message }
    }
}
