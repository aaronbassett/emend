import EmendCore
import SwiftUI

/// Placeholder root view for Phase 1.
///
/// It renders the core ABI version so the app↔`EmendCore` package link is
/// exercised at build and run time. Phase 2 (T027) replaces this with the
/// three-pane shell (sidebar | editor | info).
struct ContentView: View {
    var body: some View {
        VStack(spacing: 12) {
            Image(systemName: "doc.text")
                .font(.system(size: 48))
                .foregroundStyle(.secondary)
            Text("Emend")
                .font(.largeTitle.weight(.semibold))
            Text("Core ABI v\(EmendCore.abiVersion)")
                .font(.callout)
                .foregroundStyle(.secondary)
        }
        .frame(minWidth: 640, minHeight: 480)
        .padding()
    }
}

#Preview {
    ContentView()
}
