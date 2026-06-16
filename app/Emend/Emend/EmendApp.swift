import SwiftUI

/// Emend — a quiet, native macOS Markdown editor.
///
/// Phase 1 establishes the app target with a minimal entry point. The real
/// three-pane shell (sidebar | editor | info) and the security-scoped-bookmark
/// handshake land in Phase 2 (T027/T028).
@main
struct EmendApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
        .windowResizability(.contentMinSize)
    }
}
