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
        // The selected texture pack comes from the shared app group storage.
        // The parse runs off the main thread in the controller loader, and
        // the atlas upload waits for the parse to finish.
        let pack = await PreviewPackResolver.resourcePack()
        try rendererController.preparePreview(of: url, resourcePack: pack)
    }
}

/// Resolves the shared pack archive to a native pack handle. The resolver
/// parses one archive per resolution. The returned pack is immutable, and
/// the renderer can reuse it for this preview.
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
