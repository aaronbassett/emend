import AppKit
import EmendCore
import XCTest
@testable import Emend

/// Headless coverage for US7 typography (T127): `TypographyModel` clamps (via the
/// core) + persists, and the `Typography` resolver maps settings to the editor font
/// and preview CSS the editor + preview apply. App-hosted, not XCUITest (the
/// `EmendUITests` target doesn't exist by design).
@MainActor
final class TypographyTests: XCTestCase {
    private func freshDefaults() throws -> UserDefaults {
        try XCTUnwrap(UserDefaults(suiteName: "emend-typo-\(UUID().uuidString)"))
    }

    func testApplyClampsAndPersists() throws {
        let defaults = try freshDefaults()
        let model = TypographyModel(defaults: defaults)

        // Out-of-range values are clamped by the core (size 8...48, line 1...3,
        // paragraph 0...64); the font family is kept.
        model.apply(TypographySettings(
            fontFamily: "Menlo", fontSizePt: 999, lineHeight: 99, paragraphSpacingPt: -5
        ))
        XCTAssertLessThanOrEqual(model.settings.fontSizePt, 48)
        XCTAssertLessThanOrEqual(model.settings.lineHeight, 3.0)
        XCTAssertGreaterThanOrEqual(model.settings.paragraphSpacingPt, 0)
        XCTAssertEqual(model.settings.fontFamily, "Menlo")

        // A new model over the same defaults reads the persisted (clamped) values.
        let reloaded = TypographyModel(defaults: defaults)
        XCTAssertEqual(reloaded.settings, model.settings)
    }

    func testResolverProducesFontAndCSS() {
        let settings = TypographySettings(
            fontFamily: "Menlo", fontSizePt: 18, lineHeight: 1.6, paragraphSpacingPt: 10
        )
        XCTAssertEqual(Typography.font(for: settings).pointSize, 18, accuracy: 0.01)

        let css = Typography.previewCSS(for: settings)
        XCTAssertTrue(css.contains("font-size: 18"))
        XCTAssertTrue(css.contains("Menlo"))
        XCTAssertTrue(css.contains("line-height: 1.6"))

        // The system sentinel maps to the native font stack, not a literal name.
        let systemCSS = Typography.previewCSS(for: TypographyModel.defaultSettings)
        XCTAssertTrue(systemCSS.contains("-apple-system"))
    }
}
