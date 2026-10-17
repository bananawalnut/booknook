// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct NotebookRevisionPreview: Decodable, Identifiable {
    let id: String
    let blockId: String
    let at: UInt64
    let reader: String
    let text: String
    let styles: [InlineStyle]
}

extension DeskRepository {
    func notebookHistory(notebook: String, block: String) throws -> [NotebookRevisionPreview] {
        let raw = try required().notebookHistory(notebookId: notebook, blockId: block)
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode([NotebookRevisionPreview].self, from: Data(raw.utf8))
    }
    func restoreNotebookRevision(notebook: String, revision: String, baseUpdate: String) throws -> DeskSnapshot {
        let raw = try required().restoreNotebookRevision(notebookId: notebook, revisionId: revision, baseUpdate: baseUpdate)
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase
        return try decoder.decode(DeskSnapshot.self, from: Data(raw.utf8))
    }
}

struct NotebookHistoryView: View {
    @EnvironmentObject var model: DeskModel
    @Environment(\.dismiss) var dismiss
    let notebookId: String
    let blockId: String
    @State private var versions: [NotebookRevisionPreview] = []
    @State private var selected: String?
    @State private var loading = true
    @State private var restoring = false
    @State private var error: String?
    var preview: NotebookRevisionPreview? { versions.first { $0.id == selected } }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text("Your thought, over time").font(.system(.title2, design: .serif))
            Text("Restoring creates a new saved revision. Earlier writing remains recoverable.").foregroundStyle(.secondary)
            if loading { ProgressView().frame(maxWidth: .infinity, minHeight: 200) }
            else if versions.isEmpty { ContentUnavailableView("No earlier writing yet", systemImage: "clock.arrow.circlepath", description: Text("Committed notebook edits are saved automatically.")) }
            else {
                Picker("Saved revision", selection: $selected) {
                    ForEach(versions) { version in Text("\(Date(timeIntervalSince1970: Double(version.at)).formatted()) · \(version.reader.prefix(12))").tag(Optional(version.id)) }
                }.pickerStyle(.menu)
                if let preview {
                    ScrollView {
                        Text(AttributedString(attributedNotebook(preview.text, styles: preview.styles)))
                            .textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading).padding(16)
                    }.frame(minHeight: 240).background(nookAccent.opacity(0.04), in: RoundedRectangle(cornerRadius: 10))
                    Text("\(preview.text.count) characters · \(preview.styles.count) styled ranges").font(.caption).foregroundStyle(.secondary)
                }
            }
            if let error { Text(error).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
            HStack {
                Button("Done") { dismiss() }.disabled(restoring)
                Spacer()
                if let preview { Button(restoring ? "Restoring…" : "Restore this version") { restore(preview) }.buttonStyle(.borderedProminent).disabled(restoring || loading) }
            }
        }.padding(28).frame(minWidth: 380, idealWidth: 620, minHeight: 440)
        .task {
            do { try await model.drainSaves(); versions = try await model.repository.notebookHistory(notebook: notebookId, block: blockId); selected = versions.first?.id }
            catch { self.error = bookNookErrorMessage(error) }
            loading = false
        }
        .interactiveDismissDisabled(restoring)
    }
    private func restore(_ version: NotebookRevisionPreview) {
        restoring = true; error = nil
        Task {
            do {
                try await model.drainSaves()
                guard let block = model.state?.notebooks[notebookId]?.blocks.first(where: { $0.id == blockId }) else { throw CocoaError(.fileNoSuchFile) }
                let restored = try await model.repository.restoreNotebookRevision(notebook: notebookId, revision: version.id, baseUpdate: block.update)
                model.accept(restored); model.reloadEditorSessions(); dismiss()
            } catch { self.error = bookNookErrorMessage(error) }
            restoring = false
        }
    }
}
