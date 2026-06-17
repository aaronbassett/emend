import AppKit
import EmendCore
import SwiftUI

/// Typography preferences (US7 · FR-038): font / size / line + paragraph spacing,
/// applied live to the editor and preview. Light/dark follows the system (FR-039),
/// so there's no theme control here. Each change goes through `TypographyModel`,
/// which clamps via the core and persists.
struct TypographySettingsView: View {
    @ObservedObject var model: TypographyModel
    @Environment(\.dismiss) private var dismiss

    private let systemSentinel = "-apple-system"

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("Typography").font(.title3).bold()
            Form {
                Picker("Font", selection: binding(\.fontFamily)) {
                    Text("System").tag(systemSentinel)
                    Divider()
                    ForEach(NSFontManager.shared.availableFontFamilies, id: \.self) { family in
                        Text(family).tag(family)
                    }
                }
                Stepper(
                    "Size: \(Int(model.settings.fontSizePt)) pt",
                    value: binding(\.fontSizePt),
                    in: 8 ... 48,
                    step: 1
                )
                sliderRow("Line height", binding(\.lineHeight), 1.0 ... 3.0)
                sliderRow("Paragraph spacing", binding(\.paragraphSpacingPt), 0 ... 64)
            }
            .formStyle(.grouped)

            HStack {
                Button("Reset to Default") { model.reset() }
                Spacer()
                Button("Done") { dismiss() }.keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 420)
    }

    private func sliderRow(
        _ label: String,
        _ value: Binding<Float>,
        _ range: ClosedRange<Float>
    ) -> some View {
        HStack {
            Text(label)
            Slider(value: value, in: range)
            Text(String(format: "%.1f", value.wrappedValue))
                .monospacedDigit()
                .frame(width: 36, alignment: .trailing)
        }
    }

    /// A binding that writes a single field by applying a mutated copy through the
    /// model (which clamps + persists + republishes).
    private func binding<V>(_ keyPath: WritableKeyPath<TypographySettings, V>) -> Binding<V> {
        Binding(
            get: { model.settings[keyPath: keyPath] },
            set: { newValue in
                var updated = model.settings
                updated[keyPath: keyPath] = newValue
                model.apply(updated)
            }
        )
    }
}
