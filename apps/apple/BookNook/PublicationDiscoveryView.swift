// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct PublicationDiscoveryView: View {
    @ObservedObject var publications: PublicationsModel
    let manage: () -> Void
    @State private var presentationID = UUID()
    @State private var visibleCount = 8
    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            HStack {
                Text("External publications").font(.system(.title2, design: .serif))
                Spacer()
                Button("Manage publications", action: manage)
            }
            Text("Saved RSS and Atom articles. Publisher text is unverified and separate from friends’ shared activity and book recommendations. Only saved text is shown; links and media never load automatically.").font(.callout).foregroundStyle(.secondary)
            PublicationActivityView(publications: publications)
            HStack {
                Button("Reopen saved articles") { publications.loadSavedArticles() }.disabled(publications.busy != nil)
                Text("Offline · newest publisher dates first").font(.caption).foregroundStyle(.secondary)
            }
            if publications.loadedDiscovery && publications.entries.isEmpty {
                Text("No articles saved yet. Add a publication in Connections, then explicitly enable and refresh it. Imported settings alone do not contact a source.").foregroundStyle(.secondary)
            }
            ForEach(Array(publications.entries.prefix(visibleCount))) { item in
                Button { publications.openArticle(item, presentationID: presentationID) } label: {
                    VStack(alignment: .leading, spacing: 7) {
                        Text(verbatim: item.title).font(.system(.headline, design: .serif)).foregroundStyle(nookInk)
                        Text(verbatim: item.sourceTitle + (item.authors.isEmpty ? "" : " · " + item.authors)).font(.caption).foregroundStyle(.secondary)
                        Text(verbatim: publicationDateLabel(item.date)).font(.caption).foregroundStyle(.secondary)
                    }.frame(maxWidth: .infinity, alignment: .leading).padding(18)
                        .background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 12))
                }.buttonStyle(.plain).disabled(publications.busy != nil)
                    .accessibilityLabel("Open saved article \(item.title), from \(item.sourceTitle)")
            }
            HStack {
                if visibleCount < publications.entries.count { Button("Show more saved articles") { visibleCount = min(120, visibleCount + 8) } }
                if visibleCount > 8 { Button("Show fewer saved articles") { visibleCount = 8 } }
            }
            if publications.entries.count == 120 { Text("Showing up to 120 saved articles across your publications.").font(.caption).foregroundStyle(.secondary) }
        }.accessibilityIdentifier("publication-discovery")
            .task { if !publications.loadedDiscovery { publications.loadSavedArticles() } }
            .sheet(item: Binding(get: { publications.article?.presentationID == presentationID ? publications.article : nil }, set: { value in
                if value == nil && publications.article?.presentationID == presentationID { publications.article = nil }
            })) { PublicationArticleView(article: $0) }
    }
}

private struct PublicationArticleView: View {
    let article: PublicationArticle
    @Environment(\.dismiss) private var dismiss
    @Environment(\.openURL) private var openURL
    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            HStack { Text("Saved publication").font(.caption).foregroundStyle(.secondary); Spacer(); Button("Done") { dismiss() }.keyboardShortcut(.cancelAction) }.padding(20)
            ScrollView {
                VStack(alignment: .leading, spacing: 18) {
                    Text(verbatim: article.entry.title?.text ?? "Untitled article").font(.system(.title, design: .serif))
                    Text(verbatim: article.sourceTitle).font(.headline)
                    if !article.entry.authors.isEmpty { Text(verbatim: article.entry.authors.joined(separator: ", ")).foregroundStyle(.secondary) }
                    Text(verbatim: publicationDateLabel(article.entry.published)).font(.caption).foregroundStyle(.secondary)
                    if article.entry.updated.status != .missing { Text(verbatim: "Publisher update: " + publicationDateLabel(article.entry.updated)).font(.caption).foregroundStyle(.secondary) }
                    Text("External publisher · unverified · saved text only").font(.caption).foregroundStyle(.secondary)
                    if article.entry.conflictingIdentity { Text("The publisher reused an article ID. Each differing version is kept separately.").font(.callout).foregroundStyle(.secondary) }
                    if let summary = article.entry.summary { Text(verbatim: summary.text).textSelection(.enabled).accessibilityIdentifier("publication-literal-summary") }
                    if let content = article.entry.content { Text(verbatim: content.text).textSelection(.enabled).accessibilityIdentifier("publication-literal-content") }
                    if article.entry.omittedContent { Text("Some publisher content was omitted by the text-only reader.").font(.caption).foregroundStyle(.secondary) }
                    if !article.entry.links.isEmpty {
                        Divider()
                        Text("Open a link yourself").font(.headline)
                        Text("Opening a link uses your browser and contacts that website.").font(.caption).foregroundStyle(.secondary)
                        ForEach(Array(article.entry.links.enumerated()), id: \.offset) { _, link in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(verbatim: link.url).font(.caption).textSelection(.enabled)
                                Button("Open in browser") {
                                    if let url = URL(string: link.url), ["http", "https"].contains(url.scheme?.lowercased() ?? ""), url.host != nil { openURL(url) }
                                }.buttonStyle(.bordered).accessibilityLabel("Open \(link.url) in browser")
                            }
                        }
                    }
                    if article.entry.omittedLinks > 0 { Text(verbatim: "\(article.entry.omittedLinks) unsupported links omitted.").font(.caption).foregroundStyle(.secondary) }
                    DisclosureGroup("Saved source details") {
                        VStack(alignment: .leading, spacing: 8) {
                            Text(verbatim: "Requested: " + article.provenance.requestedURL)
                            Text(verbatim: "Received from: " + article.provenance.finalURL)
                            Text(verbatim: "Provider: " + article.provenance.providerID)
                            Text(verbatim: "Source ID: " + article.provenance.sourceID)
                            Text(verbatim: "Article ID: " + article.entry.identity.value)
                            Text(verbatim: "Variant: " + article.entry.variantKey)
                            Text(verbatim: "Raw SHA-256: " + article.rawSHA256)
                            Text(verbatim: "Normalizer: " + article.normalizerProfile)
                            if let raw = article.entry.published.raw { Text(verbatim: "Original publisher date: " + raw) }
                            if let seconds = article.entry.published.unixSeconds, let nanos = article.entry.published.nanoseconds {
                                Text(verbatim: "Publisher seconds: \(seconds) · nanoseconds: \(nanos)")
                                if nanos >= 1_000_000_000 { Text("Leap second: seconds identifies the preceding non-leap second.") }
                            }
                            Text(verbatim: "Last successful check: " + publicationCheckLabel(article.lastCheck))
                            Text(verbatim: "Body saved: " + publicationCheckLabel(article.lastBodySuccess))
                            Text(verbatim: "Publisher position: \(article.entry.firstSourceOrdinal) · Exact duplicates: \(article.entry.duplicateCount)")
                        }.font(.caption).foregroundStyle(.secondary).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(.top, 8)
                    }
                }.padding(24).frame(maxWidth: .infinity, alignment: .leading)
            }
        }.frame(minWidth: 360, idealWidth: 720, minHeight: 420, idealHeight: 720).background(nookPaper).foregroundStyle(nookInk)
    }
}
