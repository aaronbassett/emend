import Combine
import EmendCore
import Foundation

/// BYOM AI configuration (US6 · FR-035/036a, research §C5). Base URL + model are
/// non-secret and persisted in UserDefaults; the API key lives only in the
/// Keychain (`KeychainStore`). No request is ever attempted unless `isConfigured`
/// is true AND the user explicitly invokes a summary (SC-008 zero-network).
@MainActor
final class AIConfigStore: ObservableObject {
    @Published var baseURL: String
    @Published var model: String
    @Published private(set) var hasKey: Bool

    private let defaults: UserDefaults
    private let baseURLKey = "com.aaronbassett.Emend.ai.baseURL"
    private let modelKey = "com.aaronbassett.Emend.ai.model"
    /// Per-request inactivity budget and the FR-036a max-input guard.
    private static let requestTimeoutMs: UInt64 = 30000
    private static let maxInputBytes: UInt64 = 100 * 1024

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        baseURL = defaults.string(forKey: baseURLKey) ?? ""
        model = defaults.string(forKey: modelKey) ?? ""
        hasKey = KeychainStore.hasKey()
    }

    /// A usable config exists (endpoint + model + a stored key).
    var isConfigured: Bool {
        !baseURL.trimmed.isEmpty && !model.trimmed.isEmpty && hasKey
    }

    func requestConfig() -> AiRequestConfig {
        AiRequestConfig(
            baseUrl: baseURL.trimmed,
            model: model.trimmed,
            requestTimeoutMs: Self.requestTimeoutMs,
            maxInputBytes: Self.maxInputBytes
        )
    }

    /// The key, read from the Keychain immediately before a request (never cached).
    func apiKey() -> String? {
        KeychainStore.read()
    }

    /// Persist endpoint + model (UserDefaults) and, if non-empty, the key
    /// (Keychain). A blank key leaves the stored one untouched so editing other
    /// fields doesn't wipe it — use `clearKey()` to remove it deliberately.
    func save(baseURL: String, model: String, key: String) {
        self.baseURL = baseURL.trimmed
        self.model = model.trimmed
        defaults.set(self.baseURL, forKey: baseURLKey)
        defaults.set(self.model, forKey: modelKey)
        if !key.trimmed.isEmpty {
            try? KeychainStore.save(key.trimmed)
        }
        hasKey = KeychainStore.hasKey()
    }

    func clearKey() {
        KeychainStore.delete()
        hasKey = false
    }
}

private extension String {
    var trimmed: String {
        trimmingCharacters(in: .whitespacesAndNewlines)
    }
}
