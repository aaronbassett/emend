import AppKit
import EmendCore

/// Resolves core `TypographySettings` into the editor's AppKit attributes and the
/// preview's CSS (US7). One settings source, two appliers — so the editor and the
/// WKWebView preview stay visually in sync.
enum Typography {
    private static let systemSentinel = "-apple-system"

    static func font(for settings: TypographySettings) -> NSFont {
        let size = CGFloat(settings.fontSizePt)
        if settings.fontFamily == systemSentinel || settings.fontFamily.isEmpty {
            return .systemFont(ofSize: size)
        }
        // The picker supplies *family* names; `NSFont(name:)` matches PostScript/font
        // names, so resolve via a family descriptor and fall back to the system font.
        let descriptor = NSFontDescriptor(fontAttributes: [.family: settings.fontFamily])
        return NSFont(descriptor: descriptor, size: size) ?? .systemFont(ofSize: size)
    }

    static func paragraphStyle(for settings: TypographySettings) -> NSParagraphStyle {
        let style = NSMutableParagraphStyle()
        // `lineHeightMultiple` scales the font's natural line height, so it stays
        // safe for mixed font sizes (headings) — unlike a fixed point height. It
        // only approximates CSS `line-height` (font-size × number); editor and
        // preview line spacing are close, not pixel-identical (an accepted US7 gap).
        style.lineHeightMultiple = CGFloat(settings.lineHeight)
        style.paragraphSpacing = CGFloat(settings.paragraphSpacingPt)
        return style
    }

    /// CSS overriding the preview body typography. Injected after `theme.css`, so
    /// these equal-specificity `.markdown-body` rules win the cascade.
    static func previewCSS(for settings: TypographySettings) -> String {
        let family: String
        if settings.fontFamily == systemSentinel || settings.fontFamily.isEmpty {
            family = "-apple-system, BlinkMacSystemFont, system-ui, sans-serif"
        } else {
            // CSS string-context escape: backslash-escape `\` and `"` so a crafted
            // family name can't terminate the quoted value and inject CSS.
            let escaped = settings.fontFamily
                .replacingOccurrences(of: "\\", with: "\\\\")
                .replacingOccurrences(of: "\"", with: "\\\"")
            family = "\"\(escaped)\", -apple-system, sans-serif"
        }
        return """
        .markdown-body { \
        font-family: \(family); \
        font-size: \(settings.fontSizePt)px; \
        line-height: \(settings.lineHeight); }
        .markdown-body p { margin-bottom: \(settings.paragraphSpacingPt)px; }
        """
    }
}
