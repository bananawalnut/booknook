// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers

struct PublicationsSettingsView: View {
    @ObservedObject var publications: PublicationsModel
    @State private var title = ""
    @State private var address = ""
    @State private var format = "auto"
    @State private var adding = false
    @State private var importing = false
    @State private var choosingExport = false
    @State private var presentationID = UUID()
    @State private var fileDelivery: PublicationExportDelivery?
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("Publications").font(.system(.title2, design: .serif))
                Spacer()
                Menu("Manage publications") {
                    Button("Add publication URL") { adding.toggle() }
                    Button("Import settings file") { importing = true }
                    Button("Export selected settings") { choosingExport = true }
                    Button("Reload saved status") { publications.reload() }
                }.disabled(publications.busy != nil)
            }
            Text("Bring RSS or Atom publications into Discover. These are external publications, separate from verified friends and book recommendations. Opening this section never contacts a source.").font(.callout).foregroundStyle(.secondary)
            Text("Settings imports are inactive. Enabling a source lets Book Nook contact its public HTTPS address when you press refresh. There is no automatic refresh.").font(.caption).foregroundStyle(.secondary)
            PublicationActivityView(publications: publications)
            if adding {
                VStack(alignment: .leading, spacing: 12) {
                    TextField("Publication title", text: $title).textFieldStyle(.plain).padding(10).background(nookPaper, in: RoundedRectangle(cornerRadius: 8)).accessibilityIdentifier("publication-title")
                    TextField("https://example.org/feed.xml", text: $address).textFieldStyle(.plain).padding(10).background(nookPaper, in: RoundedRectangle(cornerRadius: 8)).accessibilityLabel("Publication HTTPS address").accessibilityIdentifier("publication-url")
                    Picker("Feed format", selection: $format) { Text("Detect RSS or Atom").tag("auto"); Text("RSS").tag("rss"); Text("Atom").tag("atom") }
                    HStack { Button("Cancel adding") { adding = false }; Spacer(); Button("Review new publication") { publications.add(title: title, url: address, format: format) }.buttonStyle(.bordered).disabled(address.isEmpty || publications.busy != nil) }
                }.padding(16).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
            }
            if let review = publications.importReview { PublicationImportReviewView(publications: publications, review: review).id(review.id) }
            if let review = publications.editReview {
                VStack(alignment: .leading, spacing: 12) {
                    Text(verbatim: review.title).font(.headline)
                    Text(verbatim: review.detail).font(.callout)
                    Text(review.changes ? "Saving this settings change pauses every publication on this device." : "These settings already match. Existing activations will be kept.").font(.caption).foregroundStyle(.secondary)
                    PublicationSectionPreview(section: review.candidate.feeds)
                    HStack { Button("Cancel settings change") { publications.cancelReview() }; Spacer(); Button("Save reviewed settings") { publications.acceptEdit(); adding = false }.buttonStyle(.borderedProminent).disabled(publications.busy != nil) }
                }.publicationPanel()
            }
            if let review = publications.exportReview {
                VStack(alignment: .leading, spacing: 12) {
                    Text("Review selected settings").font(.headline)
                    Text("This file contains only the sources, labels, and memberships shown below. It contains no articles, notebook, identity, or activation state. Nothing is sent to a website.").font(.callout)
                    PublicationSectionPreview(section: review.section)
                    HStack { Button("Cancel export") { publications.cancelReview() }; Spacer(); Button("Save settings file") { publications.acceptExport(presentationID: presentationID) }.buttonStyle(.borderedProminent).disabled(publications.busy != nil) }
                }.publicationPanel()
            }
            if publications.sources.isEmpty, publications.snapshot != nil {
                Text("No publications yet. Add a public feed address or import a raw feed settings file.").foregroundStyle(.secondary)
            }
            ForEach(publications.sources) { source in sourceRow(source) }
        }.padding(20).background(nookAccent.opacity(0.035), in: RoundedRectangle(cornerRadius: 14))
        .accessibilityIdentifier("publications-settings")
        .task { publications.loadIfNeeded() }
        .fileImporter(isPresented: $importing, allowedContentTypes: [.json, .data]) { result in
            switch result {
            case .success(let url): publications.importFile(url)
            case .failure(let error): if !bookNookOperationWasCancelled(error) { publications.error = bookNookErrorMessage(error) }
            }
        }
        .sheet(isPresented: $choosingExport) { PublicationExportPicker(publications: publications) }
        .onChange(of: publications.exportDelivery?.id) { _, id in
            if let delivery = publications.exportDelivery, delivery.id == id, delivery.presentationID == presentationID {
                fileDelivery = delivery
            }
        }
        .background {
            if let delivery = fileDelivery {
                PublicationExportSaver(delivery: delivery) { result in
                    let acknowledged = publications.finishExport(delivery.id)
                    if fileDelivery?.id == delivery.id { fileDelivery = nil }
                    if acknowledged, case .failure(let error) = result, !bookNookOperationWasCancelled(error) { publications.error = bookNookErrorMessage(error) }
                }.id(delivery.id)
            }
        }
    }
    private func sourceRow(_ source: PublicationSubscription) -> some View {
        let status = publications.snapshot?.statuses[source.id]
        return VStack(alignment: .leading, spacing: 9) {
            HStack {
                Text(verbatim: source.title.isEmpty ? "Untitled publication" : source.title).font(.headline)
                Spacer()
                Text(status?.locallyActive == true ? "Enabled on this device" : "Paused on this device").font(.caption).foregroundStyle(.secondary)
            }
            Text(verbatim: source.url).font(.caption).foregroundStyle(.secondary).textSelection(.enabled)
            Text(verbatim: "Last successful check: " + publicationCheckLabel(status?.lastCheckUnixSeconds) + " · Body saved: " + publicationCheckLabel(status?.lastBodySuccessUnixSeconds)).font(.caption).foregroundStyle(.secondary)
            if let error = status?.lastError { Text(verbatim: error).font(.caption).foregroundStyle(.red) }
            if status?.stale == true { Text("Saved content may be out of date. Refresh explicitly when you want to check again.").font(.caption).foregroundStyle(.secondary) }
            if !source.enabled { Text("Disabled in imported settings. This source cannot be activated until its settings are changed.").font(.caption).foregroundStyle(.secondary) }
            HStack {
                Button(status?.locallyActive == true ? "Refresh" : "Enable & refresh") { publications.refresh(source) }
                    .buttonStyle(.bordered).disabled(publications.busy != nil || !source.enabled)
                    .accessibilityLabel("\(status?.locallyActive == true ? "Refresh" : "Enable and refresh") \(source.title)")
                if status?.locallyActive == true { Button("Pause") { publications.pause(source.id) }.disabled(publications.busy != nil) }
                Spacer()
                Menu("Settings") {
                    Button(source.enabled ? "Review disabling source setting" : "Review enabling source setting") { publications.enabledSetting(source) }
                    Button(source.disclosure == "selected" ? "Review keeping settings private" : "Review export eligibility") { publications.eligibility(source) }
                    Button("Review removal", role: .destructive) { publications.remove(source.id) }
                }.disabled(publications.busy != nil)
            }
            Text(source.disclosure == "selected" ? "Eligible for a separately reviewed settings export" : "Private settings · excluded from exports").font(.caption2).foregroundStyle(.secondary)
        }.padding(.vertical, 10)
    }
}

/// Immutable per-delivery capture: a late panel completion cannot acknowledge a
/// later export owned by this or another window.
private struct PublicationExportSaver: View {
    let delivery: PublicationExportDelivery
    let completion: (Result<URL, Error>) -> Void
    @State private var presented = false
    var body: some View {
        Color.clear.frame(width: 0, height: 0)
            .fileExporter(isPresented: $presented, document: ExportDocument(data: delivery.data), contentType: .json, defaultFilename: "booknook-publications.json", onCompletion: completion)
            .onAppear { presented = true }
    }
}

struct PublicationActivityView: View {
    @ObservedObject var publications: PublicationsModel
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let busy = publications.busy { HStack { ProgressView().controlSize(.small); Text(verbatim: busy).font(.callout); Spacer(); Button("Cancel request") { publications.cancelOperation() } } }
            if let message = publications.message { Text(verbatim: message).font(.callout).foregroundStyle(.secondary).accessibilityIdentifier("publication-message") }
            if let error = publications.error { HStack(alignment: .top) { Text(verbatim: error).font(.callout).foregroundStyle(.red).textSelection(.enabled); Spacer(); Button("Dismiss") { publications.error = nil } }.accessibilityIdentifier("publication-error") }
        }
    }
}
private extension View {
    func publicationPanel() -> some View { padding(16).background(nookAccent.opacity(0.06), in: RoundedRectangle(cornerRadius: 12)) }
}
private struct PublicationSectionPreview: View {
    let section: PublicationSection
    var body: some View {
        ScrollView {
            LazyVStack(alignment: .leading, spacing: 12) {
                if section.subscriptions.isEmpty { Text("No source settings selected.").foregroundStyle(.secondary) }
                ForEach(section.subscriptions) { source in
                    VStack(alignment: .leading, spacing: 4) {
                        Text(verbatim: source.title.isEmpty ? "Untitled publication" : source.title).font(.headline)
                        Text(verbatim: source.url).font(.caption).textSelection(.enabled)
                        Text(verbatim: "Format: \(source.format) · \(source.enabled ? "enabled setting" : "disabled setting") · \(source.disclosure)").font(.caption)
                        Text(verbatim: "Labels: " + source.collectionIds.map { id in section.collections.first(where: { $0.id == id })?.title ?? id }.joined(separator: ", ")).font(.caption)
                    }
                }
                ForEach(section.collections) { collection in Text(verbatim: "Label: " + collection.title).font(.caption) }
            }.frame(maxWidth: .infinity, alignment: .leading)
        }.frame(maxHeight: 260).accessibilityIdentifier("publication-settings-preview")
    }
}
private struct PublicationImportReviewView: View {
    @ObservedObject var publications: PublicationsModel
    let review: PublicationImportPreview
    @State private var choices: [String: String] = [:]
    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Review imported settings").font(.headline)
            Text("Only raw feed settings are accepted. Nothing is activated or fetched. Existing sources are retained unless you explicitly choose an incoming interpretation of the same ID.").font(.callout)
            ForEach(review.conflicts, id: \.key) { conflict in
                VStack(alignment: .leading, spacing: 8) {
                    Text(verbatim: "Conflicting \(conflict.target): \(conflict.id)").font(.caption).textSelection(.enabled)
                    Text(verbatim: "Current: " + describe(conflict.current)).font(.callout)
                    Text(verbatim: "Incoming: " + describe(conflict.incoming)).font(.callout)
                    Picker("Interpretation for \(conflict.id)", selection: Binding(get: { choices[conflict.key] ?? "" }, set: { choices[conflict.key] = $0 })) {
                        Text("Choose explicitly").tag(""); Text("Keep current").tag("current"); Text("Use incoming").tag("incoming")
                    }
                }.padding(.vertical, 6)
            }
            if let candidate = review.candidate {
                PublicationSectionPreview(section: candidate.feeds)
                Text("A changed import pauses all publications on this device. An unchanged import keeps existing activations.").font(.caption).foregroundStyle(.secondary)
            }
            HStack {
                Button("Cancel import") { publications.cancelReview() }
                Spacer()
                if review.conflicts.isEmpty {
                    Button("Save reviewed import") { publications.acceptImport() }.buttonStyle(.borderedProminent).disabled(publications.busy != nil)
                } else {
                    Button("Review all choices") { publications.resolveImport(review.conflicts.map { .init(target: $0.target, id: $0.id, choice: choices[$0.key]!) }) }
                        .buttonStyle(.bordered).disabled(publications.busy != nil || review.conflicts.contains { choices[$0.key] == nil || choices[$0.key] == "" })
                }
            }
        }.publicationPanel()
    }
    private func describe(_ value: PublicationConflictValue) -> String {
        var fields = [value.title]
        if let url = value.url { fields.append(url) }
        if let format = value.format { fields.append("Format: " + format) }
        if let enabled = value.enabled { fields.append(enabled ? "Enabled setting" : "Disabled setting") }
        if let disclosure = value.disclosure { fields.append("Export: " + disclosure) }
        if let ids = value.collectionIds { fields.append("Label IDs: " + ids.joined(separator: ", ")) }
        return fields.joined(separator: " · ")
    }
}
private struct PublicationExportPicker: View {
    @ObservedObject var publications: PublicationsModel
    @Environment(\.dismiss) private var dismiss
    @State private var sources = Set<String>()
    @State private var labels = Set<String>()
    @State private var memberships: [String: Set<String>] = [:]
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Select settings to export").font(.system(.title2, design: .serif))
            Text("Private sources cannot be selected here. Change their export eligibility in a separate settings review first. Select labels and then each source’s label memberships explicitly.").font(.callout).foregroundStyle(.secondary)
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(publications.collections) { collection in
                        Toggle(isOn: Binding(get: { labels.contains(collection.id) }, set: { selected in
                            if selected { labels.insert(collection.id) } else { labels.remove(collection.id); for id in Array(memberships.keys) { memberships[id]?.remove(collection.id) } }
                        })) { Text(verbatim: "Include label: " + collection.title) }
                    }
                    ForEach(publications.sources) { source in
                        Toggle(isOn: Binding(get: { sources.contains(source.id) }, set: { selected in
                            if selected { sources.insert(source.id); memberships[source.id] = [] } else { sources.remove(source.id); memberships.removeValue(forKey: source.id) }
                        })) { Text(verbatim: source.title + (source.disclosure == "private" ? " · private" : "")) }.disabled(source.disclosure != "selected")
                        if sources.contains(source.id) {
                            Text(verbatim: source.url).font(.caption).foregroundStyle(.secondary)
                            ForEach(source.collectionIds, id: \.self) { id in
                                Toggle(isOn: Binding(get: { memberships[source.id]?.contains(id) == true }, set: { selected in
                                    if selected { memberships[source.id, default: []].insert(id) } else { memberships[source.id]?.remove(id) }
                                })) { Text(verbatim: "Share membership: " + (publications.collections.first(where: { $0.id == id })?.title ?? id)) }.disabled(!labels.contains(id)).padding(.leading, 16)
                            }
                        }
                    }
                }
            }.frame(minHeight: 160, maxHeight: 400)
            Text("Unused labels are omitted. No selection is made by default.").font(.caption).foregroundStyle(.secondary)
            HStack { Button("Cancel") { dismiss() }; Spacer(); Button("Review export") {
                publications.selectExport(.init(subscriptionIds: sources.sorted(), collectionIds: labels.sorted(), memberships: sources.sorted().map { .init(subscriptionId: $0, collectionIds: (memberships[$0] ?? []).sorted()) }))
                dismiss()
            }.buttonStyle(.borderedProminent).disabled(publications.busy != nil) }
        }.padding(28).frame(minWidth: 360, idealWidth: 600).background(nookPaper)
    }
}
