import XCTest
@testable import EmendCore

final class EmendCoreTests: XCTestCase {
    func testAbiVersionIsStable() {
        XCTAssertEqual(EmendCore.abiVersion, 1)
    }
}
