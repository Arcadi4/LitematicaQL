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
        // The ordered stack comes from the shared app-group handoff. The parse
        // runs off the main thread, and the atlas upload waits for it.
        switch await PreviewPackResolver.resolve() {
        case .vanilla:
            try rendererController.preparePreview(of: url)
        case let .pack(pack):
            try rendererController.preparePreview(of: url, resourcePack: pack)
        case let .unavailable(message):
            try rendererController.preparePreview(of: url, texturePackWarning: message)
        }
    }
}

/// Resolves the shared enabled stack to one native pack handle. The resolver
/// names a missing or unreadable pack instead of silently using part of the
/// configured stack.
@MainActor
private enum PreviewPackResolver {
    enum Resolution {
        case vanilla
        case pack(NativeResourcePack)
        case unavailable(String)
    }

    static func resolve() async -> Resolution {
        guard let selection = TexturePackLibrary.sharedSelection() else { return .vanilla }
        if selection.unreadable {
            return .unavailable(
                "The saved texture pack selection could not be read. This preview uses bundled vanilla resources."
            )
        }
        if !selection.missing.isEmpty {
            let names = selection.missing.map { "“\($0.name)”" }.joined(separator: ", ")
            let noun = selection.missing.count == 1 ? "texture pack" : "texture packs"
            let verb = selection.missing.count == 1 ? "is" : "are"
            return .unavailable(
                "The \(noun) \(names) \(verb) missing. This preview uses bundled vanilla resources."
            )
        }

        let sources = selection.packs.map {
            NativeResourcePack.Source(name: $0.name, url: $0.url)
        }
        do {
            let pack = try await Task.detached(priority: .userInitiated) {
                let base = try NativeResourcePack.bundled.get()
                return try NativeResourcePack.composed(from: sources, over: base)
            }.value
            return .pack(pack)
        } catch {
            return .unavailable(
                "The texture pack stack could not be loaded. This preview uses bundled vanilla resources. \(error.localizedDescription)"
            )
        }
    }
}
