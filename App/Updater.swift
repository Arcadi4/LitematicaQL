import Combine
import Sparkle
import SwiftUI

enum UpdateFeed {
    #if arch(arm64)
    static let url = "https://update.arcadia.moe/litematicaql/arm64/appcast.xml"
    #elseif arch(x86_64)
    static let url = "https://update.arcadia.moe/litematicaql/x86_64/appcast.xml"
    #else
    #error("LitematicaQL builds for arm64 and x86_64 only.")
    #endif
}

@MainActor
final class UpdaterDelegate: NSObject, SPUUpdaterDelegate {
    func feedURLString(for updater: SPUUpdater) -> String? {
        UpdateFeed.url
    }
}

@MainActor
final class CheckForUpdatesViewModel: ObservableObject {
    @Published private(set) var canCheckForUpdates = false

    init(updater: SPUUpdater) {
        updater.publisher(for: \.canCheckForUpdates)
            .assign(to: &$canCheckForUpdates)
    }
}

struct CheckForUpdatesView: View {
    @ObservedObject private var model: CheckForUpdatesViewModel
    private let updater: SPUUpdater

    init(updater: SPUUpdater) {
        self.updater = updater
        _model = ObservedObject(wrappedValue: CheckForUpdatesViewModel(updater: updater))
    }

    var body: some View {
        Button("Check for Updates…") {
            updater.checkForUpdates()
        }
        .disabled(!model.canCheckForUpdates)
    }
}
