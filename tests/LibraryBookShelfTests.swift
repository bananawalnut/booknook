// SPDX-License-Identifier: MPL-2.0
import Foundation

@main
private enum LibraryBookShelfTests {
    private struct Observation {
        let json: [String: Any]

        init(_ time: UInt64, _ shelves: String?, source: String = "42",
             connection: String? = nil, provider: String = "goodreads") {
            var fields: [String: String] = [:]
            fields["Bookshelves"] = shelves
            var value: [String: Any] = [
                "source": ["provider": provider, "id": source],
                "imported_at": time, "digest": String(repeating: "0", count: 64), "fields": fields
            ]
            if let connection { value["provider_connection_id"] = connection }
            json = value
        }
    }

    private static func check(_ name: String, _ observations: [Observation]?,
                              shelves: Set<String> = [], conflicts: Bool = false) throws {
        let orders: [[Observation]?] = observations.map { [$0, Array($0.reversed())] } ?? [nil]
        for order in orders {
            var json: [String: Any] = [
                "id": "fixture", "title": "Book", "authors": ["Writer"], "subjects": [],
                "edition_ids": ["edition"], "shelf": "read", "rating": NSNull(), "review": ""
            ]
            if let order { json["provenance"] = order.map(\.json) }
            let decoder = JSONDecoder()
            decoder.keyDecodingStrategy = .convertFromSnakeCase
            let book = try decoder.decode(LibraryBook.self, from: JSONSerialization.data(withJSONObject: json))
            precondition(book.goodreadsShelves == shelves, "\(name): unexpected memberships")
            precondition(book.hasGoodreadsShelfConflicts == conflicts, "\(name): unexpected conflict state")
        }
    }

    static func main() throws {
        try check("historical book without provenance", nil)
        try check("empty provenance", [])
        try check("missing field supplies no observation", [.init(1, "fiction"), .init(2, nil)], shelves: ["fiction"])
        try check("newer blank clears", [.init(1, "fiction"), .init(2, "")])
        try check("equal-time blank and membership", [.init(4, ""), .init(4, "fiction")],
                  shelves: ["fiction"], conflicts: true)
        try check("equal-time differing memberships", [.init(2, "old"), .init(4, "space,read"), .init(4, "fiction")],
                  shelves: ["space", "fiction"], conflicts: true)
        try check("superseded conflict disappears", [.init(3, "space"), .init(3, "fiction"), .init(4, "")])
        try check("normalized and repeated observations agree", [
            .init(5, " fiction, fiction,read, to-read ,currently-reading, orchard "),
            .init(5, "orchard,fiction"), .init(5, "orchard,fiction")
        ], shelves: ["fiction", "orchard"])
        try check("connections contribute independently", [
            .init(1, "legacy"), .init(2, "space", connection: "reader.a"),
            .init(2, "orchard", connection: "reader.b"), .init(3, "", connection: "reader.a")
        ], shelves: ["legacy", "orchard"])
        try check("source IDs and providers stay separate", [
            .init(1, "space", source: "a"), .init(2, "orchard", source: "b"),
            .init(3, "other", source: "a", provider: "kindle")
        ], shelves: ["space", "orchard"])
        print("Goodreads shelf projections converge across provenance order and retain equal-time conflicts.")
    }
}
