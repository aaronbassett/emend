import SwiftUI

/// The horizontal row of open-document tabs above the editor (research §C7).
/// Click a tab to focus it; the ✕ closes it. Hidden when nothing is open.
struct TabBarView: View {
    @ObservedObject var model: TabModel

    var body: some View {
        if !model.tabs.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 1) {
                    ForEach(model.tabs) { tab in
                        tabButton(tab)
                    }
                }
            }
            .frame(height: 30)
            .background(.bar)
        }
    }

    private func tabButton(_ tab: TabModel.Tab) -> some View {
        let isActive = tab.id == model.activeID
        return HStack(spacing: 5) {
            Text(tab.name)
                .lineLimit(1)
                .font(.callout)
                .foregroundStyle(isActive ? Color.primary : Color.secondary)
            Button {
                model.close(tab.id)
            } label: {
                Image(systemName: "xmark").font(.system(size: 9, weight: .semibold))
            }
            .buttonStyle(.plain)
            .foregroundStyle(.secondary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 5)
        .background(isActive ? Color(nsColor: .selectedContentBackgroundColor)
            .opacity(0.25) : .clear)
        .contentShape(Rectangle())
        .onTapGesture { model.activeID = tab.id }
        .help(tab.url.path(percentEncoded: false))
    }
}
