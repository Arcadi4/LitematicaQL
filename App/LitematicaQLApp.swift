import SwiftUI

@main
struct LitematicaQLApp: App {
    @StateObject private var texturePacks = TexturePackStore()

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(texturePacks)
        }
        .defaultSize(width: 900, height: 720)
        .windowToolbarStyle(.unifiedCompact)

        Settings {
            TexturePackSettings()
                .environmentObject(texturePacks)
        }
    }
}
