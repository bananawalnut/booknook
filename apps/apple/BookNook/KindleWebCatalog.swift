// SPDX-License-Identifier: MPL-2.0
import Foundation

/// Metadata-only interpretation of the pinned Kindle web search response. See
/// docs/KINDLE-WEB-CONNECTOR.md and Resources/KindleWebSourceNotice.txt.
enum KindleWebError: LocalizedError {
    case signIn, unavailable, invalidPage, pagination, limit, changed, closed
    var errorDescription: String? {
        switch self {
        case .signIn: return "Finish signing in on Amazon, then choose Import catalog. If access expired, close this sheet and sign in again."
        case .unavailable: return "Amazon did not provide the expected catalog response. This unofficial US connector may be unavailable; your library is unchanged. You can use a catalog file instead."
        case .invalidPage: return "Amazon returned an unrecognized catalog page. Nothing was imported. Use a catalog file or try again after the connector is updated."
        case .pagination: return "The catalog did not finish safely: its page token was invalid or repeated. Nothing was imported. Try again or use a catalog file."
        case .limit: return "The catalog exceeded this connector’s time or size limit (15 seconds per request, 120 seconds overall, 200 pages, 10,000 titles, 32 MiB). Nothing was imported. Use a catalog file for a larger library."
        case .changed: return "A title changed between catalog pages. Nothing was imported. Wait for Amazon’s library to settle, then try again."
        case .closed: return "The temporary Amazon session is closed. Start a new sign-in to import."
        }
    }
}

struct KindleWebItem: Decodable, Encodable, Equatable {
    let asin: String
    let title: String
    let authors: [String]
    enum CodingKeys: String, CodingKey { case asin, title, authors, position, observedAt = "observed_at" }
    init(asin: String, title: String, authors: [String]) { self.asin = asin; self.title = title; self.authors = authors }
    init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        asin = try c.decode(String.self, forKey: .asin)
        title = try c.decode(String.self, forKey: .title)
        authors = try c.decode([String].self, forKey: .authors)
    }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(asin, forKey: .asin); try c.encode(title, forKey: .title); try c.encode(authors, forKey: .authors)
        // Fetch time and percentageRead are not reading-position evidence.
        try c.encodeNil(forKey: .position); try c.encodeNil(forKey: .observedAt)
    }
}
struct KindleWebPage: Decodable {
    let itemsList: [KindleWebItem]
    let paginationToken: String?
    let libraryType: String
    let sortType: String
}
struct KindleWebCatalog: Encodable {
    let schema = "booknook.kindle-catalog.v1"
    let items: [KindleWebItem]
}
struct KindleWebCatalogResult {
    let catalog: KindleWebCatalog
    let pageCount: Int
    /// Enumeration of the endpoint, never an assertion that all purchases exist.
    let coverage = "endpoint_exhausted"
    func encodedCatalog() throws -> Data { try JSONEncoder().encode(catalog) }
}

@MainActor enum KindleWebCatalogReader {
    struct Limits {
        var pageBytes = 2 * 1024 * 1024
        var totalBytes = 32 * 1024 * 1024
        var pages = 200
        var items = 10_000
        var seconds: Double = 120
    }
    static let origin = "https://read.amazon.com"
    static func pageURL(token: String?) throws -> URL {
        if let token { try validateToken(token) }
        var url = URLComponents(string: origin + "/kindle-library/search")!
        url.queryItems = [URLQueryItem(name: "query", value: ""), .init(name: "libraryType", value: "BOOKS"),
                          .init(name: "sortType", value: "recency"), .init(name: "querySize", value: "50")]
        if let token { url.queryItems!.append(.init(name: "paginationToken", value: token)) }
        return url.url!
    }
    static func validateToken(_ token: String) throws {
        guard !token.isEmpty, token.utf8.count <= 4096,
              token.unicodeScalars.allSatisfy({ !CharacterSet.controlCharacters.contains($0) }) else { throw KindleWebError.pagination }
    }
    static func parse(_ bytes: Data, limit: Int = 2 * 1024 * 1024) throws -> KindleWebPage {
        guard !bytes.isEmpty, bytes.count <= limit else { throw KindleWebError.limit }
        let page: KindleWebPage
        do { page = try JSONDecoder().decode(KindleWebPage.self, from: bytes) }
        catch { throw KindleWebError.invalidPage }
        guard page.libraryType == "BOOKS", page.sortType == "recency", page.itemsList.count <= 50 else { throw KindleWebError.invalidPage }
        func validText(_ text: String, max: Int) -> Bool {
            !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty && text.utf8.count <= max &&
                text.unicodeScalars.allSatisfy { !CharacterSet.controlCharacters.contains($0) }
        }
        for item in page.itemsList {
            guard item.asin.utf8.count == 10, item.asin.utf8.allSatisfy({ (48...57).contains($0) || (65...90).contains($0) }),
                  validText(item.title, max: 4000), item.authors.count <= 100,
                  item.authors.allSatisfy({ validText($0, max: 2000) }) else { throw KindleWebError.invalidPage }
        }
        if let token = page.paginationToken { try validateToken(token) }
        return page
    }
    static func collect(limits: Limits = Limits(),
                        progress: (Int, Int) -> Void = { _, _ in },
                        fetch: (URL) async throws -> Data) async throws -> KindleWebCatalogResult {
        // Check the overall deadline before and after each bounded request. A
        // request already in flight can take up to its own 15-second deadline;
        // its late result is rejected, never committed as a completed catalog.
        let start = ContinuousClock.now
        var next: String?, tokens = Set<String>(), items: [KindleWebItem] = [], byASIN: [String: KindleWebItem] = [:]
        var pages = 0, bytes = 0
        repeat {
            try Task.checkCancellation()
            guard pages < limits.pages, start.duration(to: .now) < .seconds(limits.seconds) else { throw KindleWebError.limit }
            let data = try await fetch(pageURL(token: next))
            try Task.checkCancellation()
            bytes += data.count
            guard bytes <= limits.totalBytes, start.duration(to: .now) < .seconds(limits.seconds) else { throw KindleWebError.limit }
            let page = try parse(data, limit: limits.pageBytes)
            for item in page.itemsList {
                if let previous = byASIN[item.asin] {
                    guard previous == item else { throw KindleWebError.changed }
                } else {
                    guard items.count < limits.items else { throw KindleWebError.limit }
                    byASIN[item.asin] = item; items.append(item)
                }
            }
            pages += 1; progress(pages, items.count)
            next = page.paginationToken
            if let next { guard tokens.insert(next).inserted else { throw KindleWebError.pagination } }
        } while next != nil
        return KindleWebCatalogResult(catalog: KindleWebCatalog(items: items), pageCount: pages)
    }
}
