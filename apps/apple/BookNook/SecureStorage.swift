// SPDX-License-Identifier: MPL-2.0
import Foundation
import Security
#if canImport(CastaliaNative)
import CastaliaNative
#endif

/// Existing Book Nook namespace and account names remain compatible. Storage
/// implementation is the same public CastaliaNative adapter used by other hosts.
struct CustodySecureStorage: RecoveryMaterialStorage {
    let service = "org.booknook.castalia"
    private var adapter: CastaliaKeychainStorage { get throws { try CastaliaKeychainStorage(service: service) } }
    func read(account: String) throws -> Data? { try adapter.read(account: account) }
    func saveEncryptedMaterial(_ data: Data, account: String) throws { try adapter.saveEncryptedMaterial(data, account: account) }
    func remove(account: String) throws { try adapter.remove(account: account) }
}
typealias SecureStorageError = CastaliaSecureStorageError
