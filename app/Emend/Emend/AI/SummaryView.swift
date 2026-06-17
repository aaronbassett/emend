import EmendCore
import SwiftUI

/// Bridges the Rust `AiSink` foreign trait to the main actor (US6). Cross-thread
/// token/done/error callbacks hop via immutable `@Sendable` closures, like the
/// `FsObserver`/`QuickOpenSink` adapters.
final class AiSinkBridge: AiSink {
    private let token: @Sendable (String) -> Void
    private let done: @Sendable (String) -> Void
    private let fail: @Sendable (FfiError) -> Void

    init(
        token: @escaping @Sendable (String) -> Void,
        done: @escaping @Sendable (String) -> Void,
        fail: @escaping @Sendable (FfiError) -> Void
    ) {
        self.token = token
        self.done = done
        self.fail = fail
    }

    func onToken(text: String) {
        token(text)
    }

    func onDone(full: String) {
        done(full)
    }

    func onError(err: FfiError) {
        fail(err)
    }
}

/// Drives a streamed BYOM summary (US6 · FR-036). Supersede is double-guarded: a
/// new run cancels the prior `AiHandle` (FFI side) and bumps a generation so late
/// callbacks from a superseded stream are dropped.
@MainActor
final class SummaryModel: ObservableObject {
    @Published private(set) var text = ""
    @Published private(set) var isStreaming = false
    @Published private(set) var errorMessage: String?

    private var handle: AiHandle?
    private var generation = 0

    func summarize(document: OpenDocHandle, config: AiRequestConfig, apiKey: String) {
        cancel()
        generation += 1
        let gen = generation
        text = ""
        errorMessage = nil
        isStreaming = true
        let bridge = AiSinkBridge(
            token: { [weak self] chunk in Task { @MainActor in self?.append(chunk, gen) } },
            done: { [weak self] full in Task { @MainActor in self?.finish(full, gen) } },
            fail: { [weak self] err in Task { @MainActor in self?.failed(err, gen) } }
        )
        handle = summarizeDocument(h: document, cfg: config, apiKey: apiKey, sink: bridge)
    }

    func cancel() {
        handle?.cancel()
        handle = nil
        isStreaming = false
    }

    private func append(_ chunk: String, _ gen: Int) {
        guard gen == generation, isStreaming else { return }
        text += chunk
    }

    private func finish(_ full: String, _ gen: Int) {
        guard gen == generation else { return }
        if !full.isEmpty { text = full }
        isStreaming = false
        handle = nil
    }

    private func failed(_ err: FfiError, _ gen: Int) {
        guard gen == generation else { return }
        errorMessage = err.userMessage
        isStreaming = false
        handle = nil
    }
}

/// The summary sheet: a Summarize/Regenerate control, the streamed text, and
/// cancel/error states. Presented from the toolbar when AI is configured.
struct SummaryView: View {
    @ObservedObject var model: SummaryModel
    let canSummarize: Bool
    let onSummarize: () -> Void
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            HStack {
                Text("AI Summary").font(.title3).bold()
                Spacer()
                Button("Done") { dismiss() }
            }
            content
            HStack {
                if model.isStreaming {
                    Button("Stop", role: .destructive) { model.cancel() }
                    ProgressView().controlSize(.small)
                } else {
                    Button(model.text.isEmpty ? "Summarize" : "Regenerate", action: onSummarize)
                        .disabled(!canSummarize)
                }
                Spacer()
            }
        }
        .padding(20)
        .frame(width: 520, height: 420)
    }

    private var content: some View {
        ScrollView {
            if let error = model.errorMessage {
                Label(error, systemImage: "exclamationmark.triangle")
                    .foregroundStyle(.orange)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else if model.text.isEmpty, !model.isStreaming {
                Text("Generate an AI summary of the current document.")
                    .foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading)
            } else {
                Text(model.text)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(Color(nsColor: .textBackgroundColor))
        .clipShape(RoundedRectangle(cornerRadius: 6))
    }
}
