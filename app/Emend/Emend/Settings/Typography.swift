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
        return NSFont(name: settings.fontFamily, size: size) ?? .systemFont(ofSize: size)
    }

    static func paragraphStyle(for settings: TypographySettings) -> NSParagraphStyle {
        let style = NSMutableParagraphStyle()
        style.lineHeightMultiple = CGFloat(settings.lineHeight)
        style.paragraphSpacing = CGFloat(settings.paragraphSpacingPt)
        return style
    }

    /// CSS overriding the preview body typography. Injected after `theme.css`, so
    /// these equal-specificity `.markdown-body` rules win the cascade.
    static func previewCSS(for settings: TypographySettings) -> String {
        let family = if settings.fontFamily == systemSentinel || settings.fontFamily.isEmpty {
            "-apple-system, BlinkMacSystemFont, system-ui, sans-serif"
        } else {
            "\"\(settings.fontFamily)\", -apple-system, sans-serif"
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
