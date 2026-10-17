// SPDX-License-Identifier: MPL-2.0
import Foundation

/// Interpret bounded machine codes only. Raw provider bodies can contain account data,
/// credentials or HTML and must never be echoed in errors or logs.
enum ConnectorFailure {
    static func message(status: Int, body: Data?) -> String {
        let object = body.flatMap { $0.count <= 16_384 ? (try? JSONSerialization.jsonObject(with: $0)) as? [String: Any] : nil }
        switch object?["code"] as? String {
        case "issuer_denied": return "The issuer denied this approval. Check its current holder, caller, action, disclosure and expiry grants."
        case "issuer_not_configured": return "The issuer’s signing key or policy is unavailable. Ask its operator to check the private configuration."
        case "issuer_busy", "issuer_timeout", "issuer_unavailable": return "The issuer is temporarily unavailable. Retry the approval when it is ready."
        case "issuer_invalid_request", "issuer_invalid_response": return "The issuer could not validate this approval request. Check the configured endpoint and protocol version."
        case "source_not_configured": return "This companion has no live Kindle/Audible source adapter. You can import a Kindle catalog export. Install and configure a source adapter on the companion for live account access."
        case "capability_unavailable": return "This source adapter does not support that action. Refresh the companion’s capabilities or configure an adapter that does."
        case "pairing_denied": return "Companion pairing is missing, revoked, or expired. Get a fresh pairing code from the companion and pair again."
        case "source_timeout": return "The source adapter timed out. Check that it is running and retry."
        case "secure_storage_unavailable": return "The source adapter needs an unlocked native OS credential store. It will not save account secrets in plaintext files."
        case "account_exists": return "This paired device already has an account registration. Renew access or disconnect that account before connecting again."
        case "account_missing": return "Connect the Audible account for this paired device and marketplace first."
        case "session_expired": return "This login session expired or was already used. Cancel it and start a new login."
        case "registration_uncertain": return "Registration may have reached Amazon but could not be saved safely. Check Amazon’s device list and remove any new virtual Audible device before resetting this adapter. Do not repeatedly register."
        case "reset_denied": return "Only an unfinished registration without saved account credentials can be reset. Check the selected marketplace; use Disconnect account for a connected registration."
        case "source_failure", "renewal_failed": return "Source account access failed. Try Renew account access if available, or reconnect the account through its provider."
        case "invalid_source_data": return "The selected source data does not match the supported format. Check the catalog export or source adapter."
        case "kindle_snapshot_wal": return "This snapshot uses a journaling format that cannot be imported safely. Quit Kindle, then follow the companion’s local export instructions to create a complete standalone backup or catalog JSON. Keep Kindle’s live database unchanged."
        case "kindle_snapshot_schema": return "This cache does not match the supported Kindle-for-Mac metadata schema. Choose a supported standalone snapshot or export normalized catalog JSON locally."
        case "kindle_snapshot_invalid": return "The Kindle cache snapshot is incomplete, corrupt, or unsupported. Create a fresh standalone backup; the source database has not been changed."
        case "kindle_snapshot_runtime": return "This companion cannot safely read the SQLite snapshot. Update its supported SQLite runtime, or export catalog JSON locally with a compatible companion."
        case "kindle_snapshot_local_only": return "Raw Kindle snapshots are accepted only by a companion bound to loopback on this device. For a remote companion, export normalized catalog JSON locally first."
        case "payload_too_large": return "The source data exceeds the supported size limit. Choose a smaller export."
        default:
            if status == 401 || status == 403 { return "Access expired or denied. Reconnect this service in Connections." }
            if status == 503 { return "The source service is unavailable. Check the companion and its installed source adapter." }
            if (300...399).contains(status) { return "The service tried to redirect. Enter its direct HTTPS endpoint in Connections." }
            return "Service returned HTTP \(status). Check the connection or source adapter and retry."
        }
    }
}
