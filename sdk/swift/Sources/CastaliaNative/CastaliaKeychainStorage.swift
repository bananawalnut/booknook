// SPDX-License-Identifier: MIT OR Apache-2.0
import Foundation
import Security

/// Native Keychain adapter with an explicit host namespace. An optional shared
/// access group must already be configured and entitled by the embedding app.
/// No identity, provider login, or cross-application access is provisioned here.
public struct CastaliaKeychainStorage: CastaliaSecureMaterialStorage {
    public let service: String
    public let accessGroup: String?
    public init(service: String, accessGroup: String? = nil) throws {
        guard !service.isEmpty, service.utf8.count <= 256, !service.contains(where: { $0.isNewline || $0.isASCII && $0.asciiValue! < 32 }),
              accessGroup.map({ !$0.isEmpty && $0.utf8.count <= 256 }) ?? true else {
            throw CastaliaNativeError.message("Choose an explicit secure-storage service and valid optional access group.")
        }
        self.service = service; self.accessGroup = accessGroup
    }
    private func query(_ account: String) -> [String: Any] {
        var value: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: account]
        if let accessGroup { value[kSecAttrAccessGroup as String] = accessGroup }
        return value
    }
    public func read(account: String) throws -> Data? {
        var value = query(account); value[kSecReturnData as String] = true; value[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(value as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        guard status == errSecSuccess else { throw CastaliaSecureStorageError.status(status) }
        guard let data = result as? Data else { throw CastaliaNativeError.message("Secure storage returned an unexpected item type.") }
        return data
    }
    public func saveEncryptedMaterial(_ data: Data, account: String) throws {
        let match = query(account)
        let attributes: [String: Any] = [kSecValueData as String: data, kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly]
        var status = SecItemUpdate(match as CFDictionary, attributes as CFDictionary)
        if status == errSecItemNotFound {
            status = SecItemAdd(match.merging(attributes) { _, new in new } as CFDictionary, nil)
            // Another writer may create this exact service/account after the miss.
            // Updating preserves the old item if replacement fails; never delete first.
            if status == errSecDuplicateItem { status = SecItemUpdate(match as CFDictionary, attributes as CFDictionary) }
        }
        guard status == errSecSuccess else { throw CastaliaSecureStorageError.status(status) }
    }
    public func remove(account: String) throws {
        let status = SecItemDelete(query(account) as CFDictionary)
        guard status == errSecSuccess || status == errSecItemNotFound else { throw CastaliaSecureStorageError.status(status) }
    }
}
public enum CastaliaSecureStorageError: LocalizedError {
    case status(OSStatus)
    public var errorDescription: String? {
        if case .status(let code) = self { return "Secure storage: \(SecCopyErrorMessageString(code, nil) as String? ?? String(code))" }; return nil
    }
}
