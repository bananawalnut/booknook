// SPDX-License-Identifier: MPL-2.0
import Foundation

struct LibraryBook: Decodable, Identifiable {
    let id: String
    let title: String
    let authors: [String]
    let subjects: [String]
    let editionIds: [String]
    let shelf: String
    let rating: Int?
    let review: String
    private let provenance: [ShelfObservation]?

    var goodreadsShelves: Set<String> {
        Set(latestGoodreadsShelves.values.flatMap { $0.memberships.flatMap { $0 } })
    }

    var hasGoodreadsShelfConflicts: Bool {
        latestGoodreadsShelves.values.contains { $0.memberships.count > 1 }
    }

    private var latestGoodreadsShelves: [ShelfOrigin: ShelfSelection] {
        var selections: [ShelfOrigin: ShelfSelection] = [:]
        for record in provenance ?? [] {
            guard record.source.provider == "goodreads", let value = record.fields.bookshelves else { continue }
            let origin = ShelfOrigin(id: record.source.id, connection: record.providerConnectionId)
            let shelves = Set(value.split(separator: ",")
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
                .filter { !$0.isEmpty && !Self.exclusiveShelves.contains($0) })
            if let prior = selections[origin], prior.at >= record.importedAt {
                if prior.at == record.importedAt {
                    selections[origin]?.memberships.insert(shelves)
                }
            } else {
                selections[origin] = ShelfSelection(at: record.importedAt, memberships: [shelves])
            }
        }
        return selections
    }

    private static let exclusiveShelves: Set<String> = ["read", "to-read", "currently-reading"]

    private struct ShelfOrigin: Hashable {
        let id: String
        let connection: String?
    }

    private struct ShelfSelection {
        let at: UInt64
        var memberships: Set<Set<String>>
    }

    private struct ShelfObservation: Decodable {
        let source: Source
        let importedAt: UInt64
        let providerConnectionId: String?
        let fields: Fields

        struct Source: Decodable {
            let id: String
            let provider: String
        }

        struct Fields: Decodable {
            let bookshelves: String?
            enum CodingKeys: String, CodingKey { case bookshelves = "Bookshelves" }
        }
    }
}
