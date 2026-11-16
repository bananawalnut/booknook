// SPDX-License-Identifier: MPL-2.0
import Foundation
import Darwin
import Dispatch

private final class SocialFIFODeadline: @unchecked Sendable {
    private let lock = NSLock()
    private var exceeded = false
    private let timer = DispatchSource.makeTimerSource(queue: .global(qos: .userInitiated))
    init(_ fifo: URL) {
        timer.schedule(deadline: .now() + .seconds(2), repeating: .milliseconds(10))
        timer.setEventHandler { [weak self] in
            guard let self else { return }
            self.lock.lock(); self.exceeded = true; self.lock.unlock()
            let writer = Darwin.open(fifo.path, O_WRONLY | O_NONBLOCK | O_CLOEXEC)
            if writer >= 0 { _ = Darwin.close(writer) }
        }
        timer.resume()
    }
    func finish() -> Bool { timer.cancel(); lock.lock(); defer { lock.unlock() }; return exceeded }
    deinit { timer.cancel() }
}

@main struct SocialBindingImportTests {
    static func main() async throws {
        let root = FileManager.default.temporaryDirectory.appendingPathComponent("booknook-binding-file-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        func rejected(_ url: URL) async {
            do { _ = try await SocialBindingImport.readFile(url); preconditionFailure("Expected social binding file refusal") }
            catch { }
        }
        let fifo = root.appendingPathComponent("no-writer.json")
        precondition(mkfifo(fifo.path, S_IRUSR | S_IWUSR) == 0)
        let deadline = SocialFIFODeadline(fifo)
        await rejected(fifo)
        precondition(!deadline.finish(), "Binding reader blocked on a FIFO until the watchdog supplied a writer")
        let fifoLink = root.appendingPathComponent("fifo-link.json")
        try FileManager.default.createSymbolicLink(at: fifoLink, withDestinationURL: fifo)
        await rejected(fifoLink)
        await rejected(root)
        await rejected(URL(fileURLWithPath: "/dev/null"))
        await rejected(URL(string: "https://example.invalid/binding.json")!)
        await rejected(URL(string: "file://remote.example.invalid/binding.json")!)
        await rejected(URL(string: "file:///tmp/binding%00suffix")!)
        await rejected(root.appendingPathComponent("missing.json"))
        let file = root.appendingPathComponent("binding.json")
        for invalid in [Data([0xff, 0xfe]), Data([0xc3]), Data(repeating: 32, count: 300_000), Data(repeating: 32, count: 300_001)] {
            try invalid.write(to: file); await rejected(file)
        }
        let prefix = String(repeating: " ", count: 4_095) + "é😀 café 日本語\n"
        var exact = Data(prefix.utf8)
        exact.append(Data(repeating: 32, count: 299_999 - exact.count))
        try exact.write(to: file)
        let full = try await SocialBindingImport.readFile(file)
        precondition(Data(full.utf8) == exact && full.utf8.count == 299_999)
        let regularLink = root.appendingPathComponent("regular-link.json")
        try FileManager.default.createSymbolicLink(at: regularLink, withDestinationURL: file)
        let linked = try await SocialBindingImport.readFile(regularLink)
        precondition(linked == full)
        try Data().write(to: file)
        let empty = try await SocialBindingImport.readFile(file)
        precondition(empty.isEmpty, "The reader must leave empty/envelope refusal to the existing native binding validator")
        let literal = #"{"not":"an accepted binding; preserve exact bytes for native validation"}"#
        try Data(literal.utf8).write(to: file)
        let unparsed = try await SocialBindingImport.readFile(file)
        precondition(unparsed == literal)
        let cancelled = Task {
            withUnsafeCurrentTask { $0?.cancel() }
            do {
                _ = try await SocialBindingImport.readFile(URL(string: "https://example.invalid/cancel")!)
                preconditionFailure("Pre-cancelled binding read succeeded")
            } catch is CancellationError { }
            catch { preconditionFailure("Cancellation must win before URL/I/O handling") }
        }
        await cancelled.value
        print("Social-binding actual-file boundary: no-writer FIFO/watchdog, FIFO/regular symlinks, directory/device/nonlocal/missing/UTF8/strict 300 KB refusal, exact multichunk Unicode, unchanged empty/envelope handling and pre-cancellation passed.")
    }
}
