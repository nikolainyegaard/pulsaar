import SwiftUI
import AppKit

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
                    store.reload(showIndicator: true)
                }
                .keyboardShortcut("r", modifiers: .command)
            }
            CommandGroup(after: .windowArrangement) {
                Button("Return to Default Size") {
                    guard let window = NSApp.mainWindow else { return }
                    returnToDefaultSize(window: window)
                }
            }
        }
        .windowResizability(.contentMinSize)
        .defaultSize(width: 700, height: 480)
    }
}

// Reset sidebar column to ideal width, then animate window to default size.
private func returnToDefaultSize(window: NSWindow) {
    // Snap the NavigationSplitView divider to the sidebar ideal width before
    // animating, so both the column and the window settle in one motion.
    if let splitView = firstSplitView(in: window.contentView) {
        splitView.setPosition(300, ofDividerAt: 0)
    }
    var frame = window.frame
    let size = NSSize(width: 700, height: 480)
    frame.origin.y += frame.size.height - size.height
    frame.size = size
    window.setFrame(frame, display: true, animate: true)
}

// Depth-first search for the first NSSplitView in a view hierarchy.
// NavigationSplitView is backed by an NSSplitView subclass near the root.
private func firstSplitView(in view: NSView?) -> NSSplitView? {
    guard let view else { return nil }
    if let sv = view as? NSSplitView { return sv }
    for sub in view.subviews {
        if let found = firstSplitView(in: sub) { return found }
    }
    return nil
}
