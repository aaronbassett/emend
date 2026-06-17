import EmendCore
import SwiftUI

/// BYOM AI settings (US6 · FR-035, research §C5): endpoint + model + API key, with
/// a connection test. The key field is a `SecureField`; it goes straight to the
/// Keychain on Save and is never shown back. Leaving it blank keeps the stored key.
struct AISettingsView: View {
    @ObservedObject var store: AIConfigStore
    @Environment(\.dismiss) private var dismiss

    @State private var baseURL = ""
    @State private var model = ""
    @State private var key = ""
    @State private var testResult: String?
    @State private var testOK = false
    @State private var testing = false

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text("AI Provider").font(.title3).bold()
            Text(
                "Use any OpenAI-compatible endpoint (OpenAI, Ollama, llama.cpp, LM Studio, …). "
                    + "Nothing is sent until you run a summary."
            )
            .font(.callout)
            .foregroundStyle(.secondary)
            .fixedSize(horizontal: false, vertical: true)

            Form {
                TextField("Base URL", text: $baseURL, prompt: Text("https://api.openai.com/v1"))
                TextField("Model", text: $model, prompt: Text("gpt-4o-mini"))
                SecureField(
                    store.hasKey ? "API key (stored — blank keeps it)" : "API key",
                    text: $key
                )
            }
            .formStyle(.grouped)

            HStack {
                Button("Test Connection", action: runTest).disabled(testing)
                if store.hasKey {
                    Button("Clear Key", role: .destructive) { store.clearKey() }
                }
                if testing { ProgressView().controlSize(.small) }
                Spacer()
                Button("Cancel") { dismiss() }
                Button("Save") { save(); dismiss() }.keyboardShortcut(.defaultAction)
            }

            if let testResult {
                Label(
                    testResult,
                    systemImage: testOK ? "checkmark.circle" : "exclamationmark.triangle"
                )
                .font(.callout)
                .foregroundStyle(testOK ? .green : .orange)
            }
        }
        .padding(20)
        .frame(width: 460)
        .onAppear {
            baseURL = store.baseURL
            model = store.model
        }
    }

    private func save() {
        store.save(baseURL: baseURL, model: model, key: key)
        key = ""
    }

    private func runTest() {
        save() // persist current edits so the probe uses them
        guard let apiKey = store.apiKey() else {
            testOK = false
            testResult = "No API key stored."
            return
        }
        let config = store.requestConfig()
        testing = true
        testResult = nil
        Task {
            do {
                try await Task.detached { try testAiConfig(cfg: config, apiKey: apiKey) }.value
                testOK = true
                testResult = "Connection OK."
            } catch {
                testOK = false
                testResult = (error as? FfiError)?.userMessage ?? error.localizedDescription
            }
            testing = false
        }
    }
}
