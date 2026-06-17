import Foundation
import XCTest
@testable import Emend

/// Headless round-trip for the AI-key Keychain wrapper (US6 · T119). Uses a unique
/// throwaway account (the test process isn't sandboxed — same rationale as the
/// plain-bookmark workspace tests). If the environment denies Keychain access
/// (e.g. an unsigned CI binary returns `errSecMissingEntitlement`), the save
/// throws and the test skips rather than failing — the wrapper logic is still
/// exercised wherever the Keychain is available.
@MainActor
final class KeychainStoreTests: XCTestCase {
    private let account = "emend-test-\(UUID().uuidString)"

    override func tearDown() {
        KeychainStore.delete(account: account)
        super.tearDown()
    }

    func testRoundTripSaveReadUpsertDelete() throws {
        do {
            try KeychainStore.save("sk-secret-123", account: account)
        } catch let KeychainStore.KeychainError.unexpectedStatus(status) {
            throw XCTSkip("Keychain unavailable in this environment (OSStatus \(status))")
        }

        XCTAssertEqual(KeychainStore.read(account: account), "sk-secret-123")
        XCTAssertTrue(KeychainStore.hasKey(account: account))

        // Re-saving upserts (delete-then-add), not duplicates.
        try KeychainStore.save("sk-secret-456", account: account)
        XCTAssertEqual(KeychainStore.read(account: account), "sk-secret-456")

        XCTAssertTrue(KeychainStore.delete(account: account))
        XCTAssertNil(KeychainStore.read(account: account))
        XCTAssertFalse(KeychainStore.hasKey(account: account))
    }
}
