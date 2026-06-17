import EmendCore
import XCTest

/// Headless tests for the sink→AsyncStream adapters and error mapping (T025).
final class StreamingTests: XCTestCase {
    func testAiStreamYieldsTokensThenFinishes() async throws {
        let (sink, stream) = AiStream.make()
        sink.onToken(text: "Hel")
        sink.onToken(text: "lo")
        sink.onDone(full: "Hello")

        var tokens: [String] = []
        for try await token in stream {
            tokens.append(token)
        }
        XCTAssertEqual(tokens, ["Hel", "lo"])
    }

    func testAiStreamThrowsMappedErrorTerminal() async {
        let (sink, stream) = AiStream.make()
        sink.onToken(text: "partial")
        sink.onError(err: .AiTimeout)

        do {
            for try await _ in stream {}
            XCTFail("expected the stream to throw on the error terminal")
        } catch let error as FfiError {
            XCTAssertEqual(error, .AiTimeout)
        } catch {
            XCTFail("unexpected error type: \(error)")
        }
    }

    func testSearchStreamYieldsBatchesThenFinishes() async {
        let (sink, stream) = SearchStream.make()
        let hit = SearchHit(path: "/notes/b.md", name: "b.md", breadcrumb: "notes", score: 99)
        sink.onResults(batch: [hit])
        sink.onDone()

        var batches: [[SearchHit]] = []
        for await batch in stream {
            batches.append(batch)
        }
        XCTAssertEqual(batches.count, 1)
        XCTAssertEqual(batches.first?.first, hit)
    }

    func testUserMessageIncludesContext() {
        XCTAssertTrue(FfiError.NotFound(path: "a/b.md").userMessage.contains("a/b.md"))
        XCTAssertEqual(FfiError.AiNotConfigured.userMessage, "No AI provider is configured.")
    }
}
