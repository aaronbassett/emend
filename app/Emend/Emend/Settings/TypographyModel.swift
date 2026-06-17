import AppKit
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

    private static let systemSentinel = "-apple-system"

    private let handle: SettingsHandle
    private let defaults: UserDefaults
    private let key = "com.aaronbassett.Emend.typography"
    /// Installed font families (+ the system sentinel) — any other family is
    /// rejected to the sentinel, so a crafted/corrupted value can't reach the
    /// editor font or the preview CSS (defense in depth alongside CSS escaping).
    private let validFamilies: Set<String>
    /// Debounces persistence so dragging a slider doesn't write to disk per tick.
    private var saveTask: Task<Void, Never>?

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        validFamilies = Set(NSFontManager.shared.availableFontFamilies)
        handle = newSettings()
        if var stored = Self.load(from: defaults, key: key) {
            stored.fontFamily = Self.sanitize(stored.fontFamily, valid: validFamilies)
            try? handle.setTypography(settings: stored)
        }
        settings = handle.getTypography()
    }

    /// Apply new settings: sanitize the family, let the core clamp the numbers, read
    /// the clamped result back, publish it (editor/preview update live), and persist
    /// (debounced).
    func apply(_ new: TypographySettings) {
        var sanitized = new
        sanitized.fontFamily = Self.sanitize(new.fontFamily, valid: validFamilies)
        try? handle.setTypography(settings: sanitized)
        settings = handle.getTypography()
        scheduleSave()
    }

    func reset() {
        apply(Self.defaultSettings)
    }

    /// Keep only the system sentinel or an installed family; anything else → system.
    private static func sanitize(_ family: String, valid: Set<String>) -> String {
        family == systemSentinel || valid.contains(family) ? family : systemSentinel
    }

    /// Coalesce persistence to ~200 ms after the last change (persists the current
    /// published settings). The task is main-actor-isolated (created in a @MainActor
    /// method), so the UserDefaults access stays confined to the main actor.
    private func scheduleSave() {
        saveTask?.cancel()
        saveTask = Task { [weak self] in
            try? await Task.sleep(for: .milliseconds(200))
            guard !Task.isCancelled, let self else { return }
            Self.save(settings, to: defaults, key: key)
        }
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
