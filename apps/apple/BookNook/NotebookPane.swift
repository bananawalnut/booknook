// SPDX-License-Identifier: MPL-2.0
import SwiftUI

struct NotebookPane: View {
    @EnvironmentObject var model: DeskModel
    @State private var addError: String?
    @State private var adding = false
    var body: some View {
        GeometryReader { _ in
            if let notebook = model.notebook {
                ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        if !model.readerRecoverySessions.isEmpty {
                            DisclosureGroup("Reader markup needing attention") {
                                ForEach(model.readerRecoverySessions, id: \.recoveryIdentity) { session in
                                    ReaderInkPendingView(session: session)
                                }
                            }.font(.caption)
                        }
                        ForEach(Array(notebook.blocks.enumerated()), id: \.element.id) { index, block in
                            NoteBlockView(notebook: notebook, block: block, noteNumber: index + 1).id("\(block.id):\(model.editorGeneration)")
                            Rectangle().fill(nookInk.opacity(0.10)).frame(height: 1).padding(.vertical, 10).accessibilityHidden(true)
                        }
                        if let addError { Text(addError).font(.callout).foregroundStyle(.red).textSelection(.enabled) }
                        Button {
                            guard !adding else { return }; adding = true; addError = nil
                            Task {
                                defer { adding = false }
                                do { _ = try await model.createNote(notebook: notebook.id) }
                                catch { addError = bookNookErrorMessage(error) }
                            }
                        } label: {
                            Label(adding ? "Adding…" : "Add note", systemImage: "plus")
                        }.disabled(adding).accessibilityIdentifier("notebook-add-note").buttonStyle(.borderless).font(.caption).foregroundStyle(nookInk.opacity(0.6))
                            .padding(.top, 12)
                    }.padding(.horizontal, 16).padding(.top, 16).padding(.bottom, 40)
                }.accessibilityLabel("Notebook")
                    .onChange(of: model.notebookScrollTarget) { _, target in
                        if let target { withAnimation { proxy.scrollTo("\(target):\(model.editorGeneration)", anchor: .top) } }
                    }
                }
            } else { ProgressView().frame(maxWidth: .infinity, maxHeight: .infinity) }
        }.background(readingPaper)
    }
}
