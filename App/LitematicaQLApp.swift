import Sparkle
import SwiftUI

@main
struct LitematicaQLApp: App {
    @StateObject private var texturePacks = TexturePackStore()
    private let updaterDelegate: UpdaterDelegate
    private let updaterController: SPUStandardUpdaterController

    init() {
        let updaterDelegate = UpdaterDelegate()
        self.updaterDelegate = updaterDelegate
        updaterController = SPUStandardUpdaterController(
            startingUpdater: true,
            updaterDelegate: updaterDelegate,
            userDriverDelegate: nil
        )
    }

    var body: some Scene {
        WindowGroup {
            ContentView()
                .environmentObject(texturePacks)
        }
        .defaultSize(width: 900, height: 720)
        .windowToolbarStyle(.unifiedCompact)
        .commands {
            CommandGroup(after: .appInfo) {
                CheckForUpdatesView(updater: updaterController.updater)
            }
        }

        Settings {
            TexturePackSettings()
                .environmentObject(texturePacks)
        }
    }
}
