// SPDX-License-Identifier: MPL-2.0
import Foundation
import Combine

/// Recognition is an interruptible suggestion. Its generation is independent of
/// the immutable ink source and of a reviewed transcript's durable save.
@MainActor final class NotebookTranscriptionDraft: ObservableObject {
    @Published var text = ""
    @Published private(set) var method = "manual"
    @Published private(set) var isRecognizing = false
    @Published private(set) var isSaving = false
    @Published private(set) var error: String?
    private var recognition: Task<Void, Never>?
    private var generation = UUID()
    private let recognizeInk: (NotebookInkVersion) async throws -> String
    init(recognize: @escaping (NotebookInkVersion) async throws -> String = NotebookInkRecognizer.recognize) { recognizeInk = recognize }
    func recognize(_ version: NotebookInkVersion) {
        guard !isRecognizing, !isSaving, text.isEmpty else { return }
        let token = UUID(); generation = token; isRecognizing = true; error = nil
        recognition = Task {
            do {
                let result = try await recognizeInk(version)
                guard generation == token, !Task.isCancelled else { return }
                text = result; method = "on_device"
            } catch {
                guard generation == token, !Task.isCancelled else { return }
                self.error = bookNookErrorMessage(error)
            }
            guard generation == token else { return }
            isRecognizing = false; recognition = nil
        }
    }
    func cancelRecognition() {
        generation = UUID(); recognition?.cancel(); recognition = nil
        isRecognizing = false
    }
    func save(_ persist: (String, String) async throws -> Void) async -> Bool {
        guard !isSaving, !isRecognizing, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return false }
        let reviewed = text, sourceMethod = method
        isSaving = true; error = nil
        defer { isSaving = false }
        do { try await persist(reviewed, sourceMethod); return true }
        catch { self.error = bookNookErrorMessage(error); return false }
    }
}
