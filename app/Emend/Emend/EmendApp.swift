import SwiftUI

/// Emend — a quiet, native macOS Markdown editor.
///
/// Single-window app hosting the three-pane shell (`MainWindow`). The panes are
/// skeletons in Phase 2; the editor (US1), location tree (US2), and info sidebar
/// (US6) fill them in later phases.
@main
struct EmendApp: App {
    var body: some Scene {
        WindowGroup {
            MainWindow()
        }
        .windowResizability(.contentMinSize)
    }
}
