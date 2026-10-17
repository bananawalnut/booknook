// SPDX-License-Identifier: MPL-2.0
import SwiftUI

/// Sessions are explicit reader intent. Navigation and app suspension do not guess an end time.
struct ReadingSessionControl: View {
    @EnvironmentObject var model: DeskModel
    let passId: String
    @State private var isSaving = false
    @State private var error: String?

    var sessions: [ReadingSession] {
        (model.state?.sessions.values.filter { $0.passId == passId } ?? []).sorted {
            $0.startedAt == $1.startedAt ? $0.id < $1.id : $0.startedAt < $1.startedAt
        }
    }
    var active: [ReadingSession] { sessions.filter { $0.endedAt == nil } }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if active.isEmpty {
                HStack {
                    Button { save(["op": "start_session", "pass_id": passId]) } label: {
                        Label(isSaving ? "Starting session…" : "Start reading session", systemImage: "play.circle")
                    }.disabled(isSaving)
                    Text("\(sessions.count) completed in this pass").foregroundStyle(.secondary)
                }
            } else {
                ForEach(active) { session in
                    HStack {
                        Label("Reading since \(Date(timeIntervalSince1970: Double(session.startedAt)).formatted(date: .abbreviated, time: .shortened))", systemImage: "clock")
                        Spacer()
                        Button(isSaving ? "Saving…" : "Finish session") { save(["op": "end_session", "session_id": session.id]) }
                            .disabled(isSaving)
                            .accessibilityLabel("Finish reading session started \(Date(timeIntervalSince1970: Double(session.startedAt)).formatted(date: .abbreviated, time: .shortened))")
                    }
                }
                Text("Sessions stay open until you finish them, including after closing Book Nook.").foregroundStyle(.secondary)
            }
            if let error { Text(error).foregroundStyle(.red).textSelection(.enabled) }
        }.font(.caption).buttonStyle(.borderless)
    }

    private func save(_ command: [String: Any]) {
        guard !isSaving else { return }
        isSaving = true; error = nil
        Task {
            defer { isSaving = false }
            do {
                try await model.drainSaves()
                var payload = command
                payload["at"] = Int(Date().timeIntervalSince1970)
                try await model.perform(payload)
            } catch { self.error = "Could not save this reading session. \(bookNookErrorMessage(error))" }
        }
    }
}
