// SPDX-License-Identifier: MPL-2.0
import Foundation

struct PublicationExportMembership: Encodable, Sendable {
    let subscriptionId: String
    var collectionIds: [String]
}
struct PublicationExportSelection: Encodable, Sendable {
    let schema = "booknook.feed-export-selection.v1"
    var subscriptionIds: [String]
    var collectionIds: [String]
    var memberships: [PublicationExportMembership]
}
struct PublicationExportPreview: Identifiable, Sendable {
    let id: UUID
    let section: PublicationSection
}
struct PublicationExportDelivery: Identifiable, Sendable {
    let id: UUID
    let presentationID: UUID
    let data: Data
}
