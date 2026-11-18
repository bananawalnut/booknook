// SPDX-License-Identifier: MPL-2.0
import Foundation

enum PublicationFailure: LocalizedError {
    case message(String)
    var errorDescription: String? { switch self { case .message(let text): return text } }
}

// These are settings records, not imported cache/view constructors. The SDK
// validates original bytes before any decoder below sees them. Integers never
// pass through NSNumber/Double or Foundation JSON dictionaries.
struct PublicationSubscription: Codable, Equatable, Identifiable, Sendable {
    var id: String
    var url: String
    var format: String
    var title: String
    var enabled: Bool
    var collectionIds: [String]
    var disclosure: String
}
struct PublicationCollection: Codable, Equatable, Identifiable, Sendable { var id: String; var title: String }
struct PublicationPreferences: Codable, Equatable, Sendable { var order = "newest-first" }
struct PublicationSection: Codable, Equatable, Sendable {
    var schema = "castalia.feeds-section.v1"
    var version: UInt8 = 1
    var subscriptions: [PublicationSubscription] = []
    var collections: [PublicationCollection] = []
    var preferences = PublicationPreferences()
}
struct PublicationConfiguration: Codable, Equatable, Sendable {
    var schema = "booknook.feed-configuration.v1"
    var revision: UInt64 = 0
    var feeds = PublicationSection()
}
struct PublicationConflictValue: Decodable, Sendable {
    let id: String
    let title: String
    let url: String?
    let format: String?
    let enabled: Bool?
    let collectionIds: [String]?
    let disclosure: String?
}
struct PublicationConflict: Decodable, Identifiable, Sendable {
    let target: String
    let id: String
    let current: PublicationConflictValue
    let incoming: PublicationConflictValue
    var key: String { target + ":" + id }
}
struct PublicationResolution: Codable, Sendable {
    let target: String
    let id: String
    let choice: String
}
struct PublicationImportPreview: Identifiable, Sendable {
    let id: UUID
    let conflicts: [PublicationConflict]
    let candidate: PublicationConfiguration?
}
struct PublicationEditPreview: Identifiable, Sendable {
    let id: UUID
    let title: String
    let detail: String
    let candidate: PublicationConfiguration
    let changes: Bool
}
struct PublicationImportWire: Decodable {
    let schema: String
    let conflicts: [PublicationConflict]
    let candidate: PublicationConfiguration?
}

/// Exact, bounded file bytes. The SDK, rather than Foundation, decides whether
/// this is a raw feed section. A Wallet/account/envelope is never unwrapped here.
func readPublicationImport(_ url: URL) async throws -> Data {
    try Task.checkCancellation()
    guard url.isFileURL else { throw PublicationFailure.message("Choose a local feed settings file.") }
    let invalidJSONFile = PublicationFailure.message("Choose a raw feed settings JSON file no larger than 256 KiB.")
    let bytes = try await readSelectedRegularFile(
        url, maximumBytes: 262_144, priority: .utility,
        invalidFile: PublicationFailure.message("Choose a regular feed settings file no larger than 256 KiB."),
        exceedsLimit: invalidJSONFile)
    guard !bytes.isEmpty else { throw invalidJSONFile }
    return bytes
}
