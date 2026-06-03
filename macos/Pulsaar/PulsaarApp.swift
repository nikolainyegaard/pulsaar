import SwiftUI

@main
struct PulsaarApp: App {
    @State private var store = ReceiverStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environment(store)
        }
        .windowResizability(.contentMinSize)
        .defaultSize(width: 700, height: 480)
    }
}
