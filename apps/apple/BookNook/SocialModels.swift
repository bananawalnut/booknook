// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct SocialBook: Codable, Identifiable { let id: String; let title: String; let author: String }
struct SocialPost: Codable { let kind: String; let book: SocialBook?; let body: String; let stars: Int?; let readingStatus: String?; let spoiler: Bool; let replyTo: String? }
struct SocialEventModel: Decodable, Identifiable { let id: String; let actor: String; let at: UInt64; let resourceVersion: UInt64; let action: JSONValue; let delivery: JSONValue }
struct SocialPublication: Decodable { let event: SocialEventModel; let approvedAt: UInt64 }
struct SocialOutbox: Decodable { let publication: SocialPublication; let acknowledged: [String: [String]] }
struct SocialList: Codable, Identifiable { let id: String; let owner: String; let name: String; let description: String; let books: [SocialBook]; let version: UInt64; let deleted: Bool }
struct SocialGroup: Codable, Identifiable { let id: String; let owner: String; let name: String; let description: String; let `private`: Bool; let epoch: UInt64; let members: [String]; let moderators: [String]; let banned: [String]; let closed: Bool }
struct SocialInvitation: Decodable, Identifiable { let id: String; let groupId: String; let inviter: String; let invitee: String; let epoch: UInt64; let expiresAt: UInt64; let accepted: Bool }
struct SocialJoinRequest: Decodable, Identifiable { let id: String; let groupId: String; let reader: String; let epoch: UInt64; let expiresAt: UInt64 }
struct SocialFriendIntent: Decodable { let requested: Bool; let version: UInt64 }
struct SocialStateModel: Decodable { let revision: UInt64; let drafts: [String: SocialEventModel]; let outbox: [String: SocialOutbox]; let publications: [String: SocialPublication]; let friendshipIntents: [String: SocialFriendIntent]; let lists: [String: SocialList]; let groups: [String: SocialGroup]; let invitations: [String: SocialInvitation]; let joinRequests: [String: SocialJoinRequest]; let blocked: [String]; let retracted: [String]; let preferredSubjects: [String] }
struct SocialFeedItem: Decodable, Identifiable { let id: String; let actor: String; let at: UInt64; let groupId: String?; let post: SocialPost; let spoilerHidden: Bool }
struct SocialBinding: Decodable, Identifiable { let holder: String; let nostrPublicKey: String; let version: UInt64; let minimumVersion: UInt64; let validUntil: UInt64; var id: String { holder }; var active: Bool { version >= minimumVersion && validUntil > UInt64(Date().timeIntervalSince1970) } }
struct SocialSnapshot: Decodable { let state: SocialStateModel; let actor: String; let feed: [SocialFeedItem]; let bindings: [SocialBinding]; let pending: Int }
struct SocialRecommendation: Decodable, Identifiable { let bookId: String; let score: Int; let reasons: [String]; var id: String { bookId } }
struct SocialExchangeReport: Decodable { let sent: Int; let received: Int; let messages: [String] }
private func socialDecode<T: Decodable>(_ type: T.Type, _ value: String) throws -> T { let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase; return try decoder.decode(type, from: Data(value.utf8)) }
func socialJSON(_ value: [String: Any]) throws -> String { String(decoding: try JSONSerialization.data(withJSONObject: value, options: [.sortedKeys]), as: UTF8.self) }
func socialObject<T: Encodable>(_ value: T) throws -> [String: Any] { let encoder = JSONEncoder(); encoder.keyEncodingStrategy = .convertToSnakeCase; return try JSONSerialization.jsonObject(with: encoder.encode(value)) as? [String: Any] ?? [:] }
func socialShort(_ actor: String) -> String { String(actor.prefix(8)) + "…" + String(actor.suffix(6)) }
func socialAudience(isPublic: Bool, recipientCount: Int? = nil) -> String {
    if isPublic { return "Public — anyone can read this." }
    if let recipientCount { return "Private — encrypted for \(recipientCount) approved recipients." }
    return "Private — encrypted for approved recipients only."
}

extension DeskRepository {
    func socialSnapshot(reveal: Bool) throws -> SocialSnapshot { try socialDecode(SocialSnapshot.self, required().socialSnapshot(revealSpoilers: reveal)) }
    func socialPrepare(_ data: Data) throws -> String { try required().socialPrepare(actionJson: String(decoding: data, as: UTF8.self)) }
    func socialApprove(event: String, receipt: AuthorizationReceipt) throws { try required().socialApprove(eventJson: event, requestJson: receipt.requestJSON, presentationJson: receipt.presentationJSON) }
    func socialLocal(_ data: Data) throws { try required().socialLocal(commandJson: String(decoding: data, as: UTF8.self)) }
    func socialOwnedID() throws -> String { try required().socialNewOwnedId() }
    func socialGenerateKey() throws -> Data { try required().socialGenerateTransportSecret() }
    func socialPrepareBinding(_ secret: Data) throws -> String { try required().socialPrepareBinding(secret: secret) }
    func socialApproveBinding(binding: String, receipt: AuthorizationReceipt) throws -> String { try required().socialApproveBinding(bindingJson: binding, requestJson: receipt.requestJSON, presentationJson: receipt.presentationJSON) }
    func socialImportBinding(_ value: String) throws { try required().socialImportBinding(documentJson: value) }
    func socialExportBinding(_ holder: String) throws -> String { try required().socialExportBinding(holder: holder) }
    func socialRevokeBinding(_ holder: String) throws { try required().socialRevokeBinding(holder: holder) }
    func socialExchange(secret: Data, discover: Bool) async throws -> SocialExchangeReport {
        let raw = try await providerServices().exchange(providerID: socialProviderID, secret: secret, discover: discover)
        return try socialDecode(SocialExchangeReport.self, raw)
    }
    func socialRecommendations(candidates: Data, excluded: Data) async throws -> [SocialRecommendation] { try await socialDecode([SocialRecommendation].self, providerServices().recommendations(providerID: recommendationProviderID, candidatesJSON: String(decoding: candidates, as: UTF8.self), excludedJSON: String(decoding: excluded, as: UTF8.self))) }
    func socialExport(_ url: URL) throws { try required().socialExport(path: url.path) }
    func socialRestore(_ url: URL) throws { try required().socialRestore(path: url.path) }
}

@MainActor final class SocialController: ObservableObject {
    @Published var snapshot: SocialSnapshot?
    @Published var busy = false
    @Published var status = ""
    @Published var error: String?
    @Published var revealSpoilers = false
    @Published var recommendations: [SocialRecommendation] = []
    @Published var recommendationsLoading = false
    @Published var recommendationsLoaded = false
    @Published var recommendationError: String?
    @Published var catalog: [String: SocialBook] = [:]
    var actor: String { snapshot?.actor ?? "" }
    var sharingReady: Bool { !actor.isEmpty && snapshot?.bindings.contains { $0.holder == actor && $0.active } == true }
    func reload(_ model: DeskModel) async {
        do { snapshot = try await model.repository.socialSnapshot(reveal: revealSpoilers) } catch { self.error = bookNookErrorMessage(error) }
    }
    func local(_ command: [String: Any], _ model: DeskModel) async {
        do {
            try await model.repository.socialLocal(JSONSerialization.data(withJSONObject: command))
            if command["op"] as? String == "set_subjects" { status = "Interests saved privately." }
            if command["op"] as? String == "cancel_delivery" { status = "Stopped retrying this delivery. Your approval history is still saved." }
            await reload(model)
        } catch { self.error = bookNookErrorMessage(error) }
    }
    func publish(_ action: [String: Any], title: String, _ model: DeskModel, replacingDraftID: String? = nil) async {
        guard !busy else { return }; busy = true; error = nil; defer { busy = false }
        do {
            let eventJSON = try await model.repository.socialPrepare(JSONSerialization.data(withJSONObject: action))
            let event = try socialDecode(SocialEventModel.self, eventJSON)
            let rawEvent = try JSONSerialization.jsonObject(with: Data(eventJSON.utf8))
            try await model.repository.socialLocal(JSONSerialization.data(withJSONObject: ["op": "save_draft", "event": rawEvent]))
            // Keep the original recoverable until its replacement is durably saved.
            // A preparation/save failure retains the original; consent failure retains the new draft.
            if let replacingDraftID, replacingDraftID != event.id {
                try await model.repository.socialLocal(JSONSerialization.data(withJSONObject: ["op": "delete_draft", "id": replacingDraftID]))
            }
            await reload(model)
            let delivery = event.delivery.object
            let audience = socialAudience(isPublic: delivery?["kind"] as? String == "public", recipientCount: (delivery?["recipients"] as? [String])?.count)
            var statements = [audience, "Only this reviewed record leaves Book Nook. Your surrounding notebook and private history stay private."]
            statements += socialDisclosure(action: event.action.object ?? action, event: event, state: snapshot?.state)
            let receipt = try await model.authorization.authorize(payload: Data(eventJSON.utf8), action: "social.execute", resourceVersion: event.resourceVersion, disclosure: AuthorizationDisclosure(title: title, statements: statements))
            try await model.repository.socialApprove(event: eventJSON, receipt: receipt)
            status = "Approved and waiting in your outbox. Choose Send & refresh when you are ready to deliver it."
            await reload(model)
        } catch { if !bookNookOperationWasCancelled(error) { self.error = bookNookErrorMessage(error) }; await reload(model) }
    }
    func retryDraft(_ event: SocialEventModel, _ model: DeskModel) async {
        guard let action = event.action.object else { return }
        await publish(action, title: "Publish reviewed record", model, replacingDraftID: event.id)
    }
    func bind(_ model: DeskModel) async {
        guard !busy else { return }; busy = true; error = nil; defer { busy = false }
        do {
            let account = "nostr-transport-\(actor)"
            let secure = CustodySecureStorage()
            let secret: Data
            if let existing = try secure.read(account: account) { secret = existing }
            else { secret = try await model.repository.socialGenerateKey(); try secure.saveEncryptedMaterial(secret, account: account) }
            let raw = try await model.repository.socialPrepareBinding(secret)
            guard let value = try JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any], let version = value["version"] as? NSNumber else { throw AuthorizationError.message("Invalid binding contract") }
            let receipt = try await model.authorization.authorize(payload: Data(raw.utf8), action: "nostr.bind", resourceVersion: version.uint64Value, disclosure: AuthorizationDisclosure(title: "Connect your Nostr transport identity", statements: ["Publish the association between your Castalia holder and this Nostr public key.", "The transport key remains in this device's Keychain. This binding does not grant permission to publish your notes.", "Renew or revoke this binding from Book Nook. It expires after one year."]))
            _ = try await model.repository.socialApproveBinding(binding: raw, receipt: receipt)
            status = "Sharing enabled on this device. Refresh shared activity to connect."
            await reload(model)
        } catch { if !bookNookOperationWasCancelled(error) { self.error = bookNookErrorMessage(error) } }
    }
    func exchange(_ model: DeskModel, discover: Bool = true) async {
        guard !busy else { return }; busy = true; defer { busy = false }
        do {
            guard let secret = try CustodySecureStorage().read(account: "nostr-transport-\(actor)") else { throw AuthorizationError.message("Set up sharing on this device before refreshing shared activity.") }
            let report = try await model.repository.socialExchange(secret: secret, discover: discover)
            status = "\(report.sent) deliveries acknowledged · \(report.received) records received" + (report.messages.isEmpty ? "" : "\n" + report.messages.prefix(5).joined(separator: "\n"))
            await reload(model)
        } catch { self.error = bookNookErrorMessage(error) }
    }
    func recommend(_ model: DeskModel) async {
        guard !recommendationsLoading else { return }
        recommendationsLoading = true; recommendationError = nil
        defer { recommendationsLoading = false }
        do {
            var candidates: [String: [String: Any]] = [:]
            var display: [String: SocialBook] = [:]
            for book in model.books { candidates[book.id] = ["book_id": book.id, "title": book.title, "subjects": book.subjects]; display[book.id] = SocialBook(id: book.id, title: book.title, author: book.authors.joined(separator: ", ")) }
            for record in snapshot?.state.publications.values ?? Dictionary<String, SocialPublication>().values {
                guard record.event.delivery.object?["kind"] as? String == "public", snapshot?.state.retracted.contains(record.event.id) != true, snapshot?.state.blocked.contains(record.event.actor) != true else { continue }
                if let post = record.event.action.object?["post"] as? [String: Any], let book = post["book"] as? [String: Any], let id = book["id"] as? String { if candidates[id] == nil { candidates[id] = ["book_id": id, "title": book["title"] as? String ?? id, "subjects": []]; display[id] = SocialBook(id: id, title: book["title"] as? String ?? id, author: book["author"] as? String ?? "") } }
            }
            catalog = display
            recommendations = try await model.repository.socialRecommendations(candidates: JSONSerialization.data(withJSONObject: Array(candidates.values)), excluded: JSONSerialization.data(withJSONObject: model.books.filter { $0.shelf == "read" }.map(\.id)))
            recommendationsLoaded = true
        } catch { recommendationError = bookNookErrorMessage(error); recommendations = [] }
    }
}

/// Castalia's display contract excludes control/bidi characters. Render them visibly and
/// divide long prose at scalar boundaries so the full reviewed content fits each field.
private func disclosureChunks(_ label: String, _ text: String) -> [String] {
    var result: [String] = []
    var chunk = label + ": "
    for scalar in text.unicodeScalars {
        let code = scalar.value
        let unsafe = code < 32 || (127...159).contains(code) || [0x061c, 0x200e, 0x200f].contains(code) || (0x202a...0x202e).contains(code) || (0x2066...0x2069).contains(code)
        let piece: String
        if code == 10 || code == 13 { piece = " · " }
        else if code == 9 { piece = " [tab] " }
        else if unsafe { piece = " [U+" + String(code, radix: 16).uppercased() + "] " }
        else { piece = String(scalar) }
        if chunk.utf8.count + piece.utf8.count > 480 { result.append(chunk); chunk = label + " (continued): " }
        chunk += piece
    }
    if chunk != label + ": " { result.append(chunk) }
    return result
}
private func socialDisclosure(action: [String: Any], event: SocialEventModel, state: SocialStateModel?) -> [String] {
    var fields: [(String, String)] = []
    func add(_ label: String, _ text: String?) { if let text, !text.isEmpty { fields.append((label, text)) } }
    let op = action["op"] as? String ?? ""
    if let recipients = event.delivery.object?["recipients"] as? [String] { add("Encrypted recipients", recipients.joined(separator: ", ")) }
    switch op {
    case "friendship":
        add(action["requested"] as? Bool == true ? "Request or accept mutual friendship with" : "End mutual friendship with", action["peer"] as? String)
    case "publish":
        if let post = action["post"] as? [String: Any] {
            add("Publication", (post["kind"] as? String)?.replacingOccurrences(of: "_", with: " "))
            if let book = post["book"] as? [String: Any] { add("Book", [book["title"], book["author"], book["id"]].compactMap { $0 as? String }.joined(separator: " · ")) }
            add("Text", post["body"] as? String)
            if let stars = post["stars"] as? NSNumber { add("Rating", "\(stars.intValue) of 5 stars") }
            add("Reading status", (post["reading_status"] as? String)?.replacingOccurrences(of: "-", with: " "))
            add("Spoilers", post["spoiler"] as? Bool == true ? "Marked as containing spoilers" : "No spoiler warning")
            if let id = action["group_id"] as? String { add("Group", state?.groups[id]?.name ?? id) }
        }
    case "create_list", "update_list":
        add("List", action["name"] as? String); add("Description", action["description"] as? String)
        let books = (action["books"] as? [[String: Any]] ?? []).map { [$0["title"], $0["author"], $0["id"]].compactMap { $0 as? String }.joined(separator: " — ") }
        add("Books in order", books.isEmpty ? "An empty list" : books.joined(separator: " • "))
    case "delete_list":
        let id = action["id"] as? String ?? ""
        add("Delete shared list", state?.lists[id]?.name ?? id)
    case "create_group":
        add("Group", action["name"] as? String); add("Description", action["description"] as? String)
        add("Visibility", action["private"] as? Bool == true ? "Private, with encrypted invitations and discussions" : "Public discovery and public discussions")
    case "invite":
        add("Group", (action["group"] as? [String: Any])?["name"] as? String)
        add("Invite reader", action["invitee"] as? String)
        if let expiry = action["expires_at"] as? NSNumber { add("Invitation expires", Date(timeIntervalSince1970: expiry.doubleValue).formatted()) }
    case "accept_invitation":
        let id = action["invitation_id"] as? String ?? ""
        if let invitation = state?.invitations[id] { add("Accept invitation to", state?.groups[invitation.groupId]?.name ?? invitation.groupId); add("Invited by", invitation.inviter) }
        else { add("Accept invitation", id) }
        if let consent = action["admission_consent"] as? [String: Any] {
            add("Membership verification", "I allow this signed acceptance, my reader identity, and its group and expiry details to be shown to this group's current and future admitted members. My invitation and private discussions are not included.")
            add("Group identity", consent["group_id"] as? String)
            if let expiry = consent["expires_at"] as? NSNumber { add("Admission consent expires", Date(timeIntervalSince1970: expiry.doubleValue).formatted()) }
        }
    case "request_join":
        let id = action["group_id"] as? String ?? ""
        add("Request membership of", state?.groups[id]?.name ?? id)
        add("Visibility", "Your membership request and reader identity will be public. The owner may admit you during the next seven days if membership has not changed.")
    case "set_members":
        let id = action["group_id"] as? String ?? ""
        add("Change membership of", state?.groups[id]?.name ?? id)
        add("Approved members", (action["members"] as? [String] ?? []).joined(separator: ", "))
        let moderators = action["moderators"] as? [String] ?? []
        add("Moderators", moderators.isEmpty ? "Owner only" : moderators.joined(separator: ", "))
        add("Effect", "Earlier membership approvals expire. Removed members receive no subsequent private discussion.")
        if let proofs = action["admission_proofs"] as? [Any], !proofs.isEmpty { add("Membership verification", "Include \(proofs.count) signed acceptances that these readers approved for disclosure to the group's members.") }
    case "moderate":
        let id = action["group_id"] as? String ?? ""
        add("Moderate group", state?.groups[id]?.name ?? id)
        if let target = action["event_id"] as? String { add("Remove discussion", (state?.publications[target]?.event.action.object?["post"] as? [String: Any])?["body"] as? String ?? target) }
        add("Ban reader", action["ban_actor"] as? String)
    case "close_group":
        let id = action["group_id"] as? String ?? ""
        add("Close reading group", state?.groups[id]?.name ?? id)
        add("Effect", "The group stops accepting new discussions and membership activity.")
    case "retract":
        let id = action["event_id"] as? String ?? ""
        add("Retract publication", (state?.publications[id]?.event.action.object?["post"] as? [String: Any])?["body"] as? String ?? id)
        add("Effect", "Book Nook removes this record from feeds. Other readers may already have saved a copy.")
    default: add("Action", op.replacingOccurrences(of: "_", with: " "))
    }
    return fields.flatMap { disclosureChunks($0.0, $0.1) }
}
