import AppKit
import SwiftUI

/// A folder's custom icon (FR-008, C8): an SF Symbol plus an optional tint. The
/// core stores an opaque `IconId` string, so this serialises to `"symbol"` or
/// `"symbol|tint"`; `FolderIcon(serialized:)` parses it back for rendering.
struct FolderIcon: Equatable {
    enum Tint: String, CaseIterable {
        case blue, green, orange, red, purple, yellow, pink, teal, gray

        var color: Color {
            switch self {
            case .blue: .blue
            case .green: .green
            case .orange: .orange
            case .red: .red
            case .purple: .purple
            case .yellow: .yellow
            case .pink: .pink
            case .teal: .teal
            case .gray: .gray
            }
        }

        var nsColor: NSColor {
            switch self {
            case .blue: .systemBlue
            case .green: .systemGreen
            case .orange: .systemOrange
            case .red: .systemRed
            case .purple: .systemPurple
            case .yellow: .systemYellow
            case .pink: .systemPink
            case .teal: .systemTeal
            case .gray: .systemGray
            }
        }
    }

    let symbol: String
    let tint: Tint?

    var serialized: String {
        tint.map { "\(symbol)|\($0.rawValue)" } ?? symbol
    }

    init(symbol: String, tint: Tint?) {
        self.symbol = symbol
        self.tint = tint
    }

    /// Parse the stored `IconId` string back into a symbol + tint.
    init?(serialized: String?) {
        guard let serialized, !serialized.isEmpty else { return nil }
        let parts = serialized.split(separator: "|", maxSplits: 1).map(String.init)
        guard let first = parts.first else { return nil }
        symbol = first
        tint = parts.count > 1 ? Tint(rawValue: parts[1]) : nil
    }
}

/// Popover content for choosing a folder's custom icon (FR-008). Picks a tint,
/// then a symbol; `onPick(nil)` resets to the default icon.
struct FolderIconPicker: View {
    let current: FolderIcon?
    let onPick: (FolderIcon?) -> Void

    @State private var tint: FolderIcon.Tint?

    private let symbols = [
        "folder", "folder.fill", "tray.full.fill", "book.closed", "doc.text.fill",
        "star.fill", "archivebox.fill", "note.text", "bookmark.fill", "calendar",
        "briefcase.fill", "music.note", "photo.fill", "terminal.fill", "gearshape.fill"
    ]

    init(current: FolderIcon?, onPick: @escaping (FolderIcon?) -> Void) {
        self.current = current
        self.onPick = onPick
        _tint = State(initialValue: current?.tint)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Folder Icon").font(.headline)

            HStack(spacing: 6) {
                ForEach(FolderIcon.Tint.allCases, id: \.self) { swatch in
                    Circle()
                        .fill(swatch.color)
                        .frame(width: 18, height: 18)
                        .overlay(
                            Circle().strokeBorder(.primary, lineWidth: tint == swatch ? 2 : 0)
                        )
                        .onTapGesture { tint = swatch }
                }
            }

            LazyVGrid(columns: Array(repeating: GridItem(.fixed(34)), count: 5), spacing: 8) {
                ForEach(symbols, id: \.self) { symbol in
                    Button {
                        onPick(FolderIcon(symbol: symbol, tint: tint))
                    } label: {
                        Image(systemName: symbol)
                            .font(.system(size: 16))
                            .foregroundStyle(tint?.color ?? Color.secondary)
                            .frame(width: 30, height: 30)
                    }
                    .buttonStyle(.plain)
                }
            }

            Divider()
            Button("Reset to Default") { onPick(nil) }
        }
        .padding(14)
        .frame(width: 230)
    }
}
