import Combine
import EmendCore
import Foundation

/// Owns typography settings (US7 · FR-038): the core `SettingsHandle` validates +
/// clamps, this model persists Swift-side (UserDefaults) and replays into the core
/// on launch — the established app-state pattern (the core has no persistence
/// layer). `settings` is the published source of truth the editor + preview apply.
@MainActor
final class TypographyModel: ObservableObject {
    @Published private(set) var settings: TypographySettings

    /// The shipped default (matches the core's default; used for Reset).
    static let defaultSettings = TypographySettings(
        fontFamily: "-apple-system",
        fontSizePt: 14,
        lineHeight: 1.4,
        paragraphSpacingPt: 8
    )

    private let handle: SettingsHandle
    private let defaults: UserDefaults
    private let key = "com.aaronbassett.Emend.typography"

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        handle = newSettings()
        if let stored = Self.load(from: defaults, key: key) {
            try? handle.setTypography(settings: stored)
        }
        settings = handle.getTypography()
    }

    /// Apply new settings: the core clamps, we read the clamped result back, publish
    /// it (so the editor/preview update), and persist.
    func apply(_ new: TypographySettings) {
        try? handle.setTypography(settings: new)
        let clamped = handle.getTypography()
        settings = clamped
        Self.save(clamped, to: defaults, key: key)
    }

    func reset() {
        apply(Self.defaultSettings)
    }

    private static func load(from defaults: UserDefaults, key: String) -> TypographySettings? {
        guard let dict = defaults.dictionary(forKey: key),
              let family = dict["fontFamily"] as? String,
              let size = dict["fontSizePt"] as? Double,
              let lineHeight = dict["lineHeight"] as? Double,
              let paragraph = dict["paragraphSpacingPt"] as? Double
        else { return nil }
        return TypographySettings(
            fontFamily: family,
            fontSizePt: Float(size),
            lineHeight: Float(lineHeight),
            paragraphSpacingPt: Float(paragraph)
        )
    }

    private static func save(
        _ settings: TypographySettings,
        to defaults: UserDefaults,
        key: String
    ) {
        defaults.set([
            "fontFamily": settings.fontFamily,
            "fontSizePt": Double(settings.fontSizePt),
            "lineHeight": Double(settings.lineHeight),
            "paragraphSpacingPt": Double(settings.paragraphSpacingPt)
        ], forKey: key)
    }
}
