import EmendCore
import XCTest

/// Smoke test proving the app target links the local `EmendCore` package and the
/// core ABI surface is reachable. Behaviour-level tests land alongside the
/// features that introduce them (headless logic per Constitution VII).
final class EmendCoreLinkageTests: XCTestCase {
    func testCoreAbiVersionIsStable() {
        XCTAssertEqual(EmendCore.abiVersion, 1)
    }
}
