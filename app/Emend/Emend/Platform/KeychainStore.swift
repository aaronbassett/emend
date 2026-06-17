import Foundation
import Security

/// Tiny first-party Keychain wrapper for the BYOM AI key (US6 · research §C5,
/// NFR-006). Swift owns the secret's custody: the key is read immediately before
/// each AI request and handed to Rust as a transient `String` — never persisted or
/// logged Rust-side. Stored as a generic password,
/// `kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly` (device-local, no iCloud).
enum KeychainStore {
    enum KeychainError: Error, Equatable {
        case unexpectedStatus(OSStatus)
    }

    static let defaultAccount = "ai-api-key"
    private static var service: String {
        Bundle.main.bundleIdentifier ?? "com.aaronbassett.Emend"
    }

    private static func baseQuery(_ account: String) -> [String: Any] {
        [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
    }

    /// Upsert the secret (delete-then-add, so re-saving replaces cleanly).
    static func save(_ value: String, account: String = defaultAccount) throws {
        SecItemDelete(baseQuery(account) as CFDictionary)
        var attrs = baseQuery(account)
        attrs[kSecValueData as String] = Data(value.utf8)
        attrs[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let status = SecItemAdd(attrs as CFDictionary, nil)
        guard status == errSecSuccess else { throw KeychainError.unexpectedStatus(status) }
    }

    /// The stored secret, or `nil` if absent (or unreadable in this environment).
    static func read(account: String = defaultAccount) -> String? {
        var query = baseQuery(account)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var item: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &item) == errSecSuccess,
              let data = item as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Remove the secret. Succeeds whether or not an item existed.
    @discardableResult
    static func delete(account: String = defaultAccount) -> Bool {
        let status = SecItemDelete(baseQuery(account) as CFDictionary)
        return status == errSecSuccess || status == errSecItemNotFound
    }

    static func hasKey(account: String = defaultAccount) -> Bool {
        read(account: account) != nil
    }
}
