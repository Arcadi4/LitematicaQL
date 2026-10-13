import AppKit
import Quartz

@MainActor
final class PreviewViewController: NSViewController, QLPreviewingController {
    private let rendererController = SchematicMetalViewController()

    override func loadView() {
        view = NSView()
        view.wantsLayer = true

        addChild(rendererController)
        let rendererView = rendererController.view
        rendererView.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(rendererView)

        NSLayoutConstraint.activate([
            rendererView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            rendererView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            rendererView.topAnchor.constraint(equalTo: view.topAnchor),
            rendererView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        preferredContentSize = NSSize(width: 900, height: 620)
    }

    func preparePreviewOfFile(at url: URL) async throws {
        _ = view
        // Resolve the user's selected texture pack through the shared app-group
        // storage, then parse it off the main thread inside the controller's
        // loader; the atlas upload already waits for that parse to finish.
        let pack = await PreviewPackResolver.resourcePack()
        try rendererController.preparePreview(of: url, resourcePack: pack)
    }
}

/// Bridges the shared pack archive to the native pack handle. One archive is
/// parsed per resolution; the returned pack is immutable and reusable by the
/// renderer for this preview.
@MainActor
private enum PreviewPackResolver {
    static func resourcePack() async -> NativeResourcePack? {
        do {
            guard let url = TexturePackLibrary.sharedSelectedFileURL() else { return nil }
            return try await Task.detached(priority: .userInitiated) {
                try NativeResourcePack(
                    Data(contentsOf: url, options: .mappedIfSafe),
                    overlaying: NativeResourcePack.bundled.get()
                )
            }.value
        } catch {
            return nil
        }
    }
}
