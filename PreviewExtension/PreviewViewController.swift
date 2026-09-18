import AppKit
import Quartz

@MainActor
final class PreviewViewController: NSViewController, QLPreviewingController {
    private let rendererController = SchematicWebViewController()

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
        try await rendererController.preparePreview(of: url)
    }
}
