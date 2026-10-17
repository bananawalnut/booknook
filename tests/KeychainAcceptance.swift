// SPDX-License-Identifier: MPL-2.0
// Opt-in only: the runner stages the real adapter with a new synthetic service.
import Foundation
import Security
import Darwin

struct HarnessFailure: Error { let detail: String }
@main struct KeychainAcceptance {
    static func main() {
        guard CommandLine.arguments.count == 4 else { exit(64) }
        let mode = CommandLine.arguments[1]
        guard mode == "lifecycle" || mode == "cleanup" else { exit(64) }
        let account = CommandLine.arguments[2]
        let reportURL = URL(fileURLWithPath: CommandLine.arguments[3])
        let storage = CustodySecureStorage()
        guard storage.service.hasPrefix("org.booknook.qa.keychain."), account.hasPrefix("synthetic-") else { exit(65) }
        let priorReport = (try? Data(contentsOf: reportURL)).flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
        if mode == "cleanup" {
            // Cleanup requires the original harness checkpoint proving an absent, then owned record.
            let priorSteps = (priorReport?["steps"] as? [[String: Any]] ?? []).compactMap { $0["step"] as? String }
            guard priorReport?["service"] as? String == storage.service,
                  priorReport?["account"] as? String == account,
                  priorSteps.contains("initial_read_missing"), priorSteps.contains("create_begin") else { exit(65) }
        }
        var report: [String: Any] = ["service": storage.service, "account": account, "mode": mode,
            "value_classification": "synthetic opaque test bytes", "steps": [[String: Any]](),
            "result": "running", "cleanup": "no write attempted"]
        var steps = [[String: Any]]()
        @discardableResult func checkpoint(_ name: String, _ extra: [String: Any] = [:]) -> Bool {
            var entry: [String: Any] = ["step": name]
            entry.merge(extra) { _, new in new }; steps.append(entry); report["steps"] = steps
            do {
                let data = try JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys])
                try data.write(to: reportURL, options: .atomic)
                return true
            } catch { return false }
        }
        func check(_ condition: Bool, _ detail: String) throws {
            guard condition else { throw HarnessFailure(detail: detail) }
        }
        var priorInteraction = DarwinBoolean(false)
        let getStatus = SecKeychainGetUserInteractionAllowed(&priorInteraction)
        guard getStatus == errSecSuccess else {
            report["result"] = "blocked"; checkpoint("get_interaction_state_failed", ["status": getStatus]); exit(2)
        }
        let disableStatus = SecKeychainSetUserInteractionAllowed(false)
        guard disableStatus == errSecSuccess else {
            report["result"] = "blocked"; checkpoint("disable_interaction_failed", ["status": disableStatus]); exit(2)
        }
        var writeAttempted = false
        var absenceVerified = false
        var code: Int32 = 0
        checkpoint("optional_interaction_disabled", ["status": disableStatus])
        do {
            var interaction = DarwinBoolean(true)
            try check(SecKeychainGetUserInteractionAllowed(&interaction) == errSecSuccess && !interaction.boolValue,
                      "Process interaction suppression did not remain disabled")
            if mode == "cleanup" {
                checkpoint("timeout_cleanup_begin")
                try storage.remove(account: account)
                try check(try storage.read(account: account) == nil, "Cleanup absence not verified")
                report["result"] = "passed"
                report["cleanup"] = "exact owned test record removed or absent; absence verified"
                checkpoint("timeout_cleanup_succeeded")
            } else {
                checkpoint("initial_read_begin")
                try check(try storage.read(account: account) == nil, "Refusing to touch preexisting exact service/account")
                checkpoint("initial_read_missing")
                let initial = Data([0, 255, 128, 1, 2, 3]) + Data("synthetic opaque café 雨 🌿 initial".utf8)
                let replacement = Data("replacement opaque bytes".utf8) + Data((0...255).map(UInt8.init))
                try check(checkpoint("create_begin", ["byte_count": initial.count]), "Cannot record ownership checkpoint before writing")
                writeAttempted = true
                try storage.saveEncryptedMaterial(initial, account: account)
                checkpoint("create_succeeded")
                try check(try storage.read(account: account) == initial, "Created bytes did not round-trip exactly")
                checkpoint("read_created_bytes_exact")
                checkpoint("update_begin", ["byte_count": replacement.count])
                try storage.saveEncryptedMaterial(replacement, account: account)
                checkpoint("update_succeeded")
                try check(try storage.read(account: account) == replacement, "Replacement bytes did not round-trip exactly")
                checkpoint("read_updated_bytes_exact")
                checkpoint("remove_begin")
                try storage.remove(account: account)
                checkpoint("remove_succeeded")
                try check(try storage.read(account: account) == nil, "Removed record remains readable")
                absenceVerified = true
                checkpoint("read_after_remove_missing")
                try storage.remove(account: account)
                checkpoint("second_remove_idempotent")
                report["result"] = "passed"; report["cleanup"] = "exact test record removed; absence verified"
            }
        } catch {
            code = 1; report["result"] = "failed"
            if case SecureStorageError.status(let status) = error {
                report["status"] = status
                if status == errSecInteractionNotAllowed || status == errSecNotAvailable || status == errSecAuthFailed {
                    report["result"] = "blocked"; code = 2
                }
            } else if let failure = error as? HarnessFailure { report["failure"] = failure.detail }
            checkpoint("operation_stopped")
            if mode == "cleanup" { report["cleanup"] = "exact owned test account cleanup blocked; no broader action attempted" }
            if writeAttempted && !absenceVerified {
                do {
                    try storage.remove(account: account)
                    try check(try storage.read(account: account) == nil, "Cleanup absence not verified")
                    report["cleanup"] = "exact owned test record removed or absent; absence verified"
                    checkpoint("failure_cleanup_succeeded")
                } catch {
                    report["cleanup"] = "exact test account cleanup blocked; no broader action attempted"
                    if case SecureStorageError.status(let status) = error { report["cleanup_status"] = status }
                    checkpoint("failure_cleanup_blocked")
                }
            }
        }
        let restoreStatus = SecKeychainSetUserInteractionAllowed(priorInteraction.boolValue)
        report["interaction_restore_status"] = restoreStatus
        if restoreStatus != errSecSuccess { report["result"] = "failed"; code = 1 }
        checkpoint("finished")
        if let data = try? JSONSerialization.data(withJSONObject: report, options: [.prettyPrinted, .sortedKeys]) {
            FileHandle.standardOutput.write(data); FileHandle.standardOutput.write(Data([10]))
        }
        exit(code)
    }
}
