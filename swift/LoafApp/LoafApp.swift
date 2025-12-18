import SwiftUI

@main
struct LoafApp: App {
    var body: some Scene {
        WindowGroup {
            ContentView()
        }
    }
}

struct ContentView: View {
    var body: some View {
        VStack(spacing: 20) {
            Image(systemName: "externaldrive.fill")
                .font(.system(size: 64))
            Text("Loaf")
                .font(.largeTitle)
            Text("SQLite-backed virtual filesystem")
                .foregroundColor(.secondary)
        }
        .padding(40)
    }
}
