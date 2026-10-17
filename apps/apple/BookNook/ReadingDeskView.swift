// SPDX-License-Identifier: MPL-2.0
import SwiftUI
import UniformTypeIdentifiers
#if os(macOS)
import AppKit
#else
import UIKit
#endif

struct ReadingDeskView: View {
    @EnvironmentObject var model: DeskModel
    @State private var changingPass = false
    @State private var changingWorkspace = false
    @State private var showingSession = false
    @GestureState private var resizeTranslation: CGFloat = 0
    @AppStorage("booknook.notebookOnRight") private var notebookOnRight = false
    @AppStorage("booknook.notebookWidthFraction") private var notebookFraction = 0.51
    var readingPasses: [Notebook] {
        (model.state?.notebooks.values.filter { $0.bookId == model.selectedBookId } ?? []).sorted {
            let left = model.state?.passes[$0.passId]?.startedAt ?? 0, right = model.state?.passes[$1.passId]?.startedAt ?? 0
            return left == right ? $0.id < $1.id : left < right
        }
    }
    var currentPassNumber: Int? { readingPasses.firstIndex { $0.id == model.selectedNotebookId }.map { $0 + 1 } }
    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .center) {
                VStack(alignment: .leading, spacing: 5) {
                    Text(model.book?.title ?? "Reading desk").font(.system(.title2, design: .serif)).lineLimit(1)
                    Text(model.book?.authors.joined(separator: ", ") ?? "").font(.caption).foregroundStyle(.secondary).lineLimit(1)
                }
                Spacer(minLength: 12)
                if model.saving { ProgressView().controlSize(.small).accessibilityLabel("Saving locally") }
                Button { changeWorkspace { notebookOnRight.toggle() } } label: {
                    Image(systemName: "rectangle.split.2x1").frame(width: 36, height: 36)
                }
                .accessibilityLabel(notebookOnRight ? "Notebook on left" : "Notebook on right")
                .accessibilityValue(notebookOnRight ? "Notebook currently on right" : "Notebook currently on left")
                .help(notebookOnRight ? "Move notebook to the left" : "Move notebook to the right")
                .disabled(changingWorkspace)
                Menu {
                    Menu("Reading passes") {
                    ForEach(Array(readingPasses.enumerated()), id: \.element.id) { index, notebook in
                        Button { selectPass(notebook.id) } label: {
                            let date = Date(timeIntervalSince1970: Double(model.state?.passes[notebook.passId]?.startedAt ?? 0)).formatted(date: .abbreviated, time: .shortened)
                            if notebook.id == model.selectedNotebookId { Label("Pass \(index + 1) · \(date)", systemImage: "checkmark") }
                            else { Text("Pass \(index + 1) · \(date)") }
                        }
                    }
                    Divider()
                    Button("Begin a new reading pass") { beginPass() }
                    }
                    Button("Reading session…") { showingSession = true }.disabled(model.notebook == nil)
                    Button("Equal pane widths") { notebookFraction = 0.5 }
                    Divider()
                    Text(model.saving ? "Saving locally…" : "Saved locally")
                } label: { Image(systemName: "ellipsis").frame(width: 36, height: 36).accessibilityLabel("Desk options") }
                .disabled(changingPass)
            }.buttonStyle(.borderless).padding(.horizontal, 22).padding(.vertical, 14)
            Divider()
            GeometryReader { geometry in
                NotebookWorkspaceLayout(notebookOnRight: notebookOnRight, notebookFraction: splitFraction(in: geometry.size)) {
                    RetainedWorkspacePane(visible: true) { NotebookPane().environment(\.notebookPaneActive, true) }
                    resizeHandle(size: geometry.size)
                    RetainedWorkspacePane(visible: true) { ReaderPane() }
                }.clipped()
            }
        }.foregroundStyle(nookInk).background(readingPaper).environment(\.colorScheme, .light)
        .sheet(isPresented: $showingSession) {
            VStack(alignment: .leading, spacing: 24) {
                HStack { Text("Reading session").font(.title2); Spacer(); Button("Done") { showingSession = false } }
                if let passID = model.notebook?.passId { ReadingSessionControl(passId: passID).id(passID) }
            }.padding(24).frame(minWidth: 320, idealWidth: 480, minHeight: 180)
        }
    }
    private func splitFraction(in size: CGSize, translation: CGFloat? = nil) -> Double {
        let base = NotebookPaneFrames(size: size, notebookOnRight: notebookOnRight, notebookFraction: notebookFraction)
        let available = max(1, base.notebook.width + base.reader.width)
        let delta = (translation ?? resizeTranslation) / available * (notebookOnRight ? -1 : 1)
        return Double(base.notebook.width / available + delta)
    }
    private func resizeHandle(size: CGSize) -> some View {
        let frames = NotebookPaneFrames(size: size, notebookOnRight: notebookOnRight, notebookFraction: splitFraction(in: size))
        let available = max(1, frames.notebook.width + frames.reader.width)
        let current = Double(frames.notebook.width / available)
        return RoundedRectangle(cornerRadius: 2).fill(nookInk.opacity(0.22)).frame(width: 3, height: 32)
            .frame(maxWidth: .infinity, maxHeight: .infinity).contentShape(Rectangle())
            .gesture(DragGesture(minimumDistance: 0, coordinateSpace: .global).updating($resizeTranslation) { drag, translation, _ in
                translation = drag.translation.width
            }.onEnded { drag in
                let next = NotebookPaneFrames(size: size, notebookOnRight: notebookOnRight,
                    notebookFraction: splitFraction(in: size, translation: drag.translation.width))
                notebookFraction = Double(next.notebook.width / available)
            })
            .accessibilityElement().accessibilityLabel("Resize notebook and book")
            .accessibilityValue("Notebook \(Int(current * 100)) percent")
            .accessibilityAdjustableAction { direction in
                let fraction = current + (direction == .increment ? 0.05 : -0.05)
                let next = NotebookPaneFrames(size: size, notebookOnRight: notebookOnRight, notebookFraction: fraction)
                notebookFraction = Double(next.notebook.width / available)
            }
            .accessibilityAction(named: Text("Equal pane widths")) { notebookFraction = 0.5 }
            .help("Drag to resize the notebook and book")
    }
    func changeWorkspace(_ change: @escaping () -> Void) {
        guard !changingWorkspace else { return }
        changingWorkspace = true
        Task {
            defer { changingWorkspace = false }
            do { try await model.drainSaves(allowJournaledInk: true); change() }
            catch { model.error = bookNookErrorMessage(error) }
        }
    }
    func selectPass(_ id: String) {
        guard !changingPass, id != model.selectedNotebookId else { return }
        changingPass = true
        Task {
            defer { changingPass = false }
            do { try await model.drainSaves(allowJournaledInk: true); model.selectedNotebookId = id; model.reloadEditorSessions() }
            catch { model.error = bookNookErrorMessage(error) }
        }
    }
    func beginPass() {
        guard !changingPass, let book = model.selectedBookId else { return }
        changingPass = true
        Task {
            defer { changingPass = false }
            do {
                try await model.drainSaves(allowJournaledInk: true)
                let existing = Set(model.state?.notebooks.keys.map { $0 } ?? [])
                try await model.perform(["op": "start_pass", "book_id": book, "at": Int(Date().timeIntervalSince1970)])
                guard let created = model.state?.notebooks.values.first(where: { $0.bookId == book && !existing.contains($0.id) }) else { throw CocoaError(.fileReadUnknown) }
                model.selectedNotebookId = created.id; model.reloadEditorSessions()
            } catch { model.error = bookNookErrorMessage(error) }
        }
    }

}
