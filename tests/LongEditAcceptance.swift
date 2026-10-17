// SPDX-License-Identifier: MPL-2.0
// Opt-in headless native editor/autosave acceptance. Invoke scripts/test-long-edit.py.
import Foundation
import AppKit
import SwiftUI
import SQLite3
import CryptoKit

struct MeasurementFailure: Error { let message: String }
func uptime() -> Double { Double(DispatchTime.now().uptimeNanoseconds) / 1_000_000_000 }
func distribution(_ values: [Double]) -> [String: Any] {
    let ordered = values.sorted()
    guard !ordered.isEmpty else { return ["count": 0] }
    func percentile(_ fraction: Double) -> Double { ordered[min(ordered.count - 1, Int(ceil(Double(ordered.count) * fraction)) - 1)] * 1000 }
    return ["count": ordered.count, "minimum_ms": ordered[0] * 1000,
            "median_ms": percentile(0.5), "p95_ms": percentile(0.95), "maximum_ms": ordered.last! * 1000]
}
actor JournalObserver {
    private var db: OpaquePointer?
    init(path: String) throws {
        guard sqlite3_open_v2(path, &db, SQLITE_OPEN_READONLY | SQLITE_OPEN_NOMUTEX, nil) == SQLITE_OK else {
            throw MeasurementFailure(message: "Cannot open isolated journal observer")
        }
        sqlite3_busy_timeout(db, 250)
    }
    deinit { sqlite3_close(db) }
    func sequence(_ session: String) throws -> Int64 {
        var statement: OpaquePointer?
        // The editor creates the journal tables on its first staged write.
        let status = sqlite3_prepare_v2(db, "SELECT max(sequence) FROM edit_branches WHERE session=?1", -1, &statement, nil)
        guard status == SQLITE_OK else { return 0 }
        defer { sqlite3_finalize(statement) }
        let transient = unsafeBitCast(-1, to: sqlite3_destructor_type.self)
        _ = session.withCString { sqlite3_bind_text(statement, 1, $0, -1, transient) }
        guard sqlite3_step(statement) == SQLITE_ROW else { throw MeasurementFailure(message: "Journal observation failed") }
        return sqlite3_column_int64(statement, 0)
    }
}

@main struct LongEditAcceptance {
    @MainActor static func main() async {
        guard CommandLine.arguments.count == 2 else { exit(64) }
        let output = URL(fileURLWithPath: CommandLine.arguments[1], isDirectory: true)
        var report: [String: Any] = ["result": "running"]
        func writeReport() {
            if let bytes = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
                try? bytes.write(to: output.appendingPathComponent("result.json"), options: .atomic)
            }
        }
        func require(_ value: Bool, _ message: String) throws { if !value { throw MeasurementFailure(message: message) } }
        let app = NSApplication.shared; app.setActivationPolicy(.prohibited)
        var heartbeat: Task<Void, Never>?
        var observationTask: Task<Void, Never>?
        do {
            let deskRoot = output.appendingPathComponent("synthetic-desk", isDirectory: true)
            try FileManager.default.createDirectory(at: deskRoot, withIntermediateDirectories: true)
            setenv("BOOKNOOK_DATA_DIR", deskRoot.path, 1)
            let model = DeskModel(); await model.open()
            try require(model.error == nil && model.books.isEmpty, "Expected empty isolated desk")
            let added = try await model.perform(["op": "add_book", "title": "Synthetic long-edit acceptance", "author": "Synthetic writer", "at": 1])
            let bookID = added!.object!["book_id"] as! String
            try await model.selectBook(bookID)
            let notebookID = model.notebook!.id, blockID = model.notebook!.blocks[0].id
            let body = "😀 café — 日本語\n" + String(repeating: "A character's meaning changes with each reading.\n", count: 2400)
            let marks = (0..<120).map { index in InlineStyle(startUtf16: 20 + index * 700, lengthUtf16: 8, bold: index % 2 == 0, italic: index % 2 != 0, underline: false) }
            var seedTimes: [Double] = []
            for index in 0..<12 {
                let prior = model.state!.notebooks[notebookID]!.blocks[0]
                let start = uptime()
                try await model.perform(["op": "edit_rich_block", "notebook_id": notebookID, "block_id": blockID,
                    "base_update": prior.update, "text": body + "Saved seed revision \(index).\n", "styles": marks.map(\.dictionary), "at": index + 2])
                seedTimes.append(uptime() - start)
            }
            let seeded = model.state!.notebooks[notebookID]!, block = seeded.blocks[0]
            let seededRevisionIDs = Set(seeded.revisions.map(\.id))
            let baseUpdate = block.update, editorSession = UUID().uuidString
            var boundText = block.text, boundStyles = block.styles ?? []
            var callbackCount = 0, callbacksOnMainThread = true
            let native = NativeNotebookEditor(text: Binding(get: { boundText }, set: { boundText = $0 }),
                styles: Binding(get: { boundStyles }, set: { boundStyles = $0 }), formatRequest: nil) { value, styles in
                    callbackCount += 1; callbacksOnMainThread = callbacksOnMainThread && Thread.isMainThread
                    model.saveText(notebook: notebookID, block: blockID, session: editorSession,
                                   baseUpdate: baseUpdate, text: value, styles: styles)
                }
            let coordinator = NativeNotebookEditor.Coordinator(native)
            let editor = NotebookTextView(usingTextLayoutManager: true)
            editor.frame = CGRect(x: 0, y: 0, width: 600, height: 300)
            editor.isRichText = true; editor.allowsUndo = true; editor.isVerticallyResizable = true
            editor.isHorizontallyResizable = false; editor.textContainer?.widthTracksTextView = true
            editor.textStorage?.setAttributedString(attributedNotebook(boundText, styles: boundStyles))
            coordinator.lastText = boundText; coordinator.lastStyles = boundStyles; editor.delegate = coordinator
            editor.changed = { [weak editor, weak coordinator] in if let editor { coordinator?.capture(editor) } }
            let observer = try JournalObserver(path: deskRoot.appendingPathComponent("booknook.sqlite").path)
            var expected = boundText
            var inputTimes: [Double] = [], acknowledgementTimes: [Double] = [], flushTimes: [Double] = []
            var heartbeatGaps: [Double] = []
            var bursts: [[String: Any]] = []
            var starts: [Double] = [], scheduled: [Double] = [], intervals: [Double] = []
            var observedSequences: [(sequence: Int64, time: Double)] = []
            var observerFailure: Error?
            observationTask = Task { @MainActor in
                var lastObserved: Int64 = 0
                while !Task.isCancelled {
                    do {
                        let sequence = try await observer.sequence(editorSession)
                        if sequence > lastObserved { observedSequences.append((sequence, uptime())); lastObserved = sequence }
                        try await Task.sleep(for: .milliseconds(5))
                    } catch is CancellationError { break }
                    catch { observerFailure = error; break }
                }
            }
            heartbeat = Task { @MainActor in
                var previous = uptime()
                while !Task.isCancelled {
                    do { try await Task.sleep(for: .milliseconds(10)) } catch { break }
                    let now = uptime(); heartbeatGaps.append(now - previous); previous = now
                }
            }
            let startEditing = uptime()
            for burst in 0..<12 {
                let burstStart = uptime()
                for offset in 0..<10 {
                    let target = burstStart + Double(offset) * 0.05
                    let remaining = target - uptime()
                    if remaining > 0 { try await Task.sleep(for: .seconds(remaining)) }
                    let index = burst * 10 + offset
                    let old = expected as NSString
                    let location: Int
                    if index % 3 == 0 { location = 0 }
                    else if index % 3 == 1 {
                        let half = old.length / 2
                        location = old.range(of: "\n", options: [], range: NSRange(location: half, length: old.length - half)).location
                    } else { location = old.length }
                    let inserted = index % 10 == 9 ? "🌿" : (index % 7 == 0 ? "雨" : "x")
                    let replacement = NSMutableString(string: expected); replacement.insert(inserted, at: location)
                    expected = replacement as String
                    editor.setSelectedRange(NSRange(location: location, length: 0))
                    let start = uptime()
                    scheduled.append(target); starts.append(start)
                    if offset > 0 { intervals.append(start - starts[starts.count - 2]) }
                    editor.insertText(inserted, replacementRange: NSRange(location: NSNotFound, length: 0))
                    inputTimes.append(uptime() - start)
                    // A missing native delegate must fail, never be replaced by a manual capture.
                    try require(callbackCount == index + 1, "Editor did not deliver exactly one edit intent")
                    try require(editor.string == expected && boundText == expected, "Native editor or binding lost Unicode text")
                }
                // Only the latest complete intent for each burst must be durable;
                // intermediate staged versions may intentionally coalesce.
                let journalDeadline = uptime() + 10
                while (observedSequences.last?.sequence ?? 0) < Int64((burst + 1) * 10) {
                    try require(uptime() < journalDeadline, "Latest burst journal acknowledgement timed out")
                    try require(model.error == nil && observerFailure == nil, "Autosave or observer error")
                    try await Task.sleep(for: .milliseconds(1))
                }
                let finalInputStart = starts.last!
                let journalObservedAt = observedSequences.first { $0.sequence >= Int64((burst + 1) * 10) }!.time
                acknowledgementTimes.append(journalObservedAt - finalInputStart)
                let flushStart = uptime(), deadline = flushStart + 30
                while model.saving {
                    try require(uptime() < deadline, "Autosave idle flush timed out")
                    try require(model.error == nil, model.error ?? "Autosave error")
                    try await Task.sleep(for: .milliseconds(10))
                }
                let flushTime = uptime() - flushStart; flushTimes.append(flushTime)
                let notebook = model.state!.notebooks[notebookID]!, saved = notebook.blocks[0]
                try require(saved.text == expected && saved.styles == portableStyles(editor.attributedString()), "Saved text/style differs from editor")
                try require(seededRevisionIDs.isSubset(of: Set(notebook.revisions.map(\.id))), "Seeded revision history was lost")
                bursts.append(["burst": burst + 1, "history_versions": notebook.revisions.count,
                               "revision": model.state!.revision, "idle_wait_ms": flushTime * 1000,
                               "last_input_to_observed_journal_ms": (journalObservedAt - finalInputStart) * 1000,
                               "last_input_to_saved_ms": (uptime() - finalInputStart) * 1000])
                print("burst \(burst + 1): idle flush \(Int(flushTime * 1000))ms, retained history \(notebook.revisions.count)")
                fflush(stdout)
            }
            let editingSeconds = uptime() - startEditing
            heartbeat?.cancel(); await heartbeat?.value; heartbeat = nil
            observationTask?.cancel(); await observationTask?.value; observationTask = nil
            let drainStart = uptime(); try await model.drainSaves(); let drainSeconds = uptime() - drainStart
            let expectedStyles = portableStyles(editor.attributedString())
            let finalNotebook = model.state!.notebooks[notebookID]!
            let reopenStart = uptime(); let reopened = DeskModel(); await reopened.open(); let reopenSeconds = uptime() - reopenStart
            try require(reopened.error == nil, reopened.error ?? "Reopen error")
            let recovered = reopened.state!.notebooks[notebookID]!
            try require(recovered.blocks[0].text == expected && recovered.blocks[0].styles == expectedStyles, "Reopen lost text or rich styles")
            try require(Set(recovered.revisions.map(\.id)) == Set(finalNotebook.revisions.map(\.id)), "Reopen altered history")
            let journalSequence = try await observer.sequence(editorSession)
            let finalHash = SHA256.hash(data: Data(expected.utf8)).map { String(format: "%02x", $0) }.joined()
            report = ["result": "passed", "initial_utf16_units": (block.text as NSString).length,
                "initial_utf8_bytes": block.text.utf8.count, "initial_history_versions": seeded.revisions.count,
                "initial_style_runs": (block.styles ?? []).count, "edits": 120, "bursts": bursts,
                "nominal_independent_cadence_ms": 50, "journal_observer_sleep_ms": 5,
                "within_burst_actual_start_interval": distribution(intervals),
                "scheduled_start_lateness": distribution(zip(starts, scheduled).map { max(0, $0 - $1) }),
                "observed_distinct_journal_sequences": observedSequences.count,
                "observed_sequences": observedSequences.map { $0.sequence },
                "journal_assertion": "Only latest sequence per burst; observed jumps may coalesce intermediate intents",
                "input_samples": zip(starts, scheduled).enumerated().map { index, times in
                    ["sequence": index + 1, "start_offset_ms": (times.0 - startEditing) * 1000,
                     "scheduled_offset_ms": (times.1 - startEditing) * 1000, "synchronous_ms": inputTimes[index] * 1000] as [String: Any]
                }, "heartbeat_target_ms": 10,
                "callback_count": callbackCount, "manual_capture_fallback_count": 0,
                "callbacks_on_main_thread": callbacksOnMainThread, "last_durable_sequence": journalSequence,
                "synchronous_insert_and_delegate": distribution(inputTimes),
                "last_input_to_observed_latest_burst_commit": distribution(acknowledgementTimes),
                "idle_wait_including_remaining_300ms_debounce": distribution(flushTimes),
                "main_actor_heartbeat_gap": distribution(heartbeatGaps),
                "seed_history_command": distribution(seedTimes), "editing_wall_seconds": editingSeconds,
                "explicit_final_drain_ms": drainSeconds * 1000, "model_reopen_ms": reopenSeconds * 1000,
                "final_history_versions": recovered.revisions.count, "final_style_runs": expectedStyles.count,
                "final_text_sha256": finalHash, "final_utf8_bytes": expected.utf8.count,
                "integrity": "Latest sequence per burst durable; exact Unicode text and portable styles after every flush and reopen; seeded history IDs retained; no autosave errors",
                "limits": "No visible window, physical typing, SwiftUI body rendering/word-count/layout, IME/Pencil, account or Keychain operations. Main-actor scheduling probe and read-only SQLite polling add measurement overhead. One bounded synthetic workload; no performance threshold or guarantee."]
            writeReport()
            print("Long-edit actual editor/autosave acceptance passed; \(editingSeconds) seconds; report saved")
        } catch {
            heartbeat?.cancel(); observationTask?.cancel()
            report["result"] = "failed"
            report["failure"] = (error as? MeasurementFailure)?.message ?? String(describing: error)
            writeReport(); print("Long-edit acceptance failed: \(report["failure"]!)"); exit(1)
        }
    }
}
