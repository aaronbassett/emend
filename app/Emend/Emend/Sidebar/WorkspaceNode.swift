import EmendCore
import Foundation

/// One row in the workspace sidebar tree. A reference type because
/// `NSOutlineView` identifies rows by object identity; children are listed
/// lazily from the Rust core on expansion (research §C6).
final class WorkspaceNode {
    enum Kind: Equatable {
        case favorites // synthetic "Favorites" group row
        case location(id: UInt64)
        case folder
        case file
    }

    let url: URL
    let name: String
    let kind: Kind
    /// `nil` until first listed; files never gain children.
    var children: [WorkspaceNode]?

    init(url: URL, name: String, kind: Kind) {
        self.url = url
        self.name = name
        self.kind = kind
    }

    var path: String {
        url.path(percentEncoded: false)
    }

    var isExpandable: Bool {
        kind != .file
    }
}

/// Bridges the Rust watcher's `DocObserver` callbacks — delivered on a background
/// thread — to a `@Sendable` closure. Holds only an immutable closure, so it is
/// safely `Sendable`.
final class FsObserver: DocObserver {
    private let onChange: @Sendable (ChangeEvent) -> Void

    init(onChange: @escaping @Sendable (ChangeEvent) -> Void) {
        self.onChange = onChange
    }

    func onDerivedChanged() {}
    func onFsChange(change: ChangeEvent) {
        onChange(change)
    }
}
