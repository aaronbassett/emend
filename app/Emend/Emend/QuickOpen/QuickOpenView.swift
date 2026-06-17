import EmendCore
import SwiftUI

/// The ⌘P Quick Open palette (US3 · FR-017): a focused search field over a
/// ranked, streaming result list. ↑/↓ move the selection, Return opens it, Esc
/// closes. Each result shows the file name with its folder breadcrumb.
struct QuickOpenView: View {
    @ObservedObject var model: QuickOpenModel
    @FocusState private var fieldFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            searchField
            Divider()
            resultsList
        }
        .frame(width: 560)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: 12))
        .overlay(RoundedRectangle(cornerRadius: 12).strokeBorder(Color(.separatorColor)))
        .shadow(radius: 24, y: 8)
        .onAppear { fieldFocused = true }
        .onKeyPress(.downArrow) { model.moveSelection(by: 1); return .handled }
        .onKeyPress(.upArrow) { model.moveSelection(by: -1); return .handled }
        .onKeyPress(.escape) { model.dismiss(); return .handled }
    }

    private var searchField: some View {
        HStack(spacing: 8) {
            Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
            TextField("Search files by name or path…", text: $model.query)
                .textFieldStyle(.plain)
                .font(.title3)
                .focused($fieldFocused)
                .onSubmit { model.openSelected() }
                .onChange(of: model.query) { _, _ in model.runQuery() }
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 12)
    }

    @ViewBuilder private var resultsList: some View {
        if model.results.isEmpty {
            placeholder
        } else {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(spacing: 0) {
                        ForEach(Array(model.results.enumerated()), id: \.offset) { index, hit in
                            QuickOpenRow(hit: hit, isSelected: index == model.selection)
                                .id(index)
                                .contentShape(Rectangle())
                                .onTapGesture {
                                    model.selection = index
                                    model.openSelected()
                                }
                        }
                    }
                }
                .frame(maxHeight: 360)
                .onChange(of: model.selection) { _, sel in
                    withAnimation(.easeOut(duration: 0.1)) { proxy.scrollTo(sel, anchor: .center) }
                }
            }
        }
    }

    @ViewBuilder private var placeholder: some View {
        let trimmed = model.query.trimmingCharacters(in: .whitespacesAndNewlines)
        let message = trimmed.isEmpty ? "Type to search your workspace." : "No matches."
        Text(message)
            .font(.callout)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.horizontal, 14)
            .padding(.vertical, 16)
    }
}

/// One result row: file name, with its folder breadcrumb beneath.
private struct QuickOpenRow: View {
    let hit: SearchHit
    let isSelected: Bool

    var body: some View {
        HStack(spacing: 10) {
            Image(systemName: "doc.text")
                .foregroundStyle(isSelected ? Color.white : Color.secondary)
            VStack(alignment: .leading, spacing: 1) {
                Text(hit.name)
                    .font(.body)
                    .foregroundStyle(isSelected ? Color.white : Color.primary)
                if !hit.breadcrumb.isEmpty {
                    Text(hit.breadcrumb)
                        .font(.caption)
                        .foregroundStyle(isSelected ? Color.white.opacity(0.8) : Color.secondary)
                        .lineLimit(1)
                        .truncationMode(.middle)
                }
            }
            Spacer()
        }
        .padding(.horizontal, 14)
        .padding(.vertical, 7)
        .background(isSelected ? Color.accentColor : Color.clear)
    }
}
