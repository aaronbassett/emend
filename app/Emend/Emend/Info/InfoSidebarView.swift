import EmendCore
import SwiftUI

/// The info sidebar (US6 · FR-029/030/031a): live document stats, task completion,
/// and a clickable heading outline that scrolls the editor. Pragmatic-UI; the
/// numbers + outline come from the core via `InfoModel`.
struct InfoSidebarView: View {
    @ObservedObject var model: InfoModel
    /// Invoked with a 1-based source line when an outline heading is clicked.
    let onSelectHeading: (Int) -> Void

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 16) {
                if let stats = model.stats {
                    statsSection(stats)
                }
                outlineSection
                Spacer(minLength: 0)
            }
            .padding()
        }
        .frame(minWidth: 220)
    }

    private func statsSection(_ stats: DocStats) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text("Document")
                .font(.headline)
            statRow("Words", "\(stats.words)")
            statRow("Characters", "\(stats.chars)")
            statRow(
                "Reading time",
                stats.readingMinutes <= 1 ? "1 min" : "\(stats.readingMinutes) min"
            )
            if stats.tasksTotal > 0 {
                statRow("Tasks", "\(stats.tasksDone) of \(stats.tasksTotal)")
                ProgressView(value: Double(stats.tasksDone), total: Double(stats.tasksTotal))
                    .progressViewStyle(.linear)
            }
        }
    }

    private func statRow(_ label: String, _ value: String) -> some View {
        HStack {
            Text(label).foregroundStyle(.secondary)
            Spacer()
            Text(value).monospacedDigit()
        }
        .font(.callout)
    }

    @ViewBuilder private var outlineSection: some View {
        if model.outline.isEmpty {
            Text("No headings yet.")
                .font(.callout)
                .foregroundStyle(.tertiary)
        } else {
            VStack(alignment: .leading, spacing: 2) {
                Text("Outline")
                    .font(.headline)
                    .padding(.bottom, 2)
                ForEach(Array(model.outline.enumerated()), id: \.offset) { _, item in
                    Button {
                        onSelectHeading(Int(item.line))
                    } label: {
                        Text(item.title)
                            .font(.callout)
                            .lineLimit(1)
                            .truncationMode(.tail)
                            .padding(.leading, CGFloat(max(0, Int(item.level) - 1)) * 12)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                    .buttonStyle(.plain)
                    .help(item.title)
                }
            }
        }
    }
}
