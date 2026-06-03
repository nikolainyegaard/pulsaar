import SwiftUI

@main
struct PulsaarApp: App {
    @State private var store = ReceiverStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(store)
        }
        .commands {
            CommandMenu("Receivers") {
                Button("Force Refresh") {
                    store.reload()
                }
                .keyboardShortcut("r", modifiers: .command)
            }
        }
        .windowResizability(.contentMinSize)
        .defaultSize(width: 700, height: 480)
    }
}
