import AppKit
import Foundation

/// App-scoped security-scoped bookmarks for user-granted folder "locations"
/// (research §A4). Swift owns the bookmark lifecycle; Rust only ever receives a
/// resolved path while Swift holds the scope open — the sandbox extension is
/// process-wide, so Rust's reads/writes/watches succeed inside the scope.
///
/// The bookmark options are parameterised so the resolution/staleness logic is
/// unit-testable with plain (non-security-scoped) bookmarks outside the sandbox;
/// the app uses `.withSecurityScope` (the defaults).
enum SecurityScopedBookmarks {
    struct Resolved {
        let url: URL
        /// Non-nil when the bookmark was stale and has been re-created — persist
        /// this in place of the old data.
        let refreshedData: Data?
    }

    /// Create a bookmark for a user-granted folder.
    static func makeBookmark(
        for url: URL,
        options: URL.BookmarkCreationOptions = [.withSecurityScope]
    ) throws -> Data {
        try url.bookmarkData(
            options: options,
            includingResourceValuesForKeys: nil,
            relativeTo: nil
        )
    }

    /// Resolve a bookmark to a URL, transparently re-creating it if stale.
    static func resolve(
        _ data: Data,
        resolutionOptions: URL.BookmarkResolutionOptions = [.withSecurityScope],
        creationOptions: URL.BookmarkCreationOptions = [.withSecurityScope]
    ) throws -> Resolved {
        var isStale = false
        let url = try URL(
            resolvingBookmarkData: data,
            options: resolutionOptions,
            relativeTo: nil,
            bookmarkDataIsStale: &isStale
        )
        guard isStale else { return Resolved(url: url, refreshedData: nil) }
        let refreshed = try withScope(url) { try makeBookmark(for: $0, options: creationOptions) }
        return Resolved(url: url, refreshedData: refreshed)
    }

    /// Run `body` with the security scope open for `url`, always balancing the
    /// `start`/`stop` calls. A non-security-scoped URL simply runs `body`
    /// directly (the start call returns `false` and no stop is needed).
    @discardableResult
    static func withScope<T>(_ url: URL, perform body: (URL) throws -> T) rethrows -> T {
        let granted = url.startAccessingSecurityScopedResource()
        defer {
            if granted { url.stopAccessingSecurityScopedResource() }
        }
        return try body(url)
    }

    /// Prompt the user to grant a folder location, returning its bookmark (or
    /// `nil` if cancelled). Main-actor isolated: `NSOpenPanel` is UI.
    @MainActor
    static func promptForFolder() throws -> Data? {
        let panel = NSOpenPanel()
        panel.canChooseDirectories = true
        panel.canChooseFiles = false
        panel.allowsMultipleSelection = false
        panel.prompt = "Add Location"
        guard panel.runModal() == .OK, let url = panel.url else { return nil }
        return try makeBookmark(for: url)
    }
}
