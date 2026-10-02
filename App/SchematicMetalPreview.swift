import SwiftUI

struct SchematicMetalPreview: NSViewControllerRepresentable {
    let fileURL: URL
    let resourcePack: NativeResourcePack?
    @Binding var errorMessage: String?

    func makeCoordinator() -> Coordinator { Coordinator() }

    func makeNSViewController(context: Context) -> SchematicMetalViewController {
        SchematicMetalViewController()
    }

    func updateNSViewController(_ controller: SchematicMetalViewController, context: Context) {
        guard context.coordinator.lastURL != fileURL || context.coordinator.lastPack !== resourcePack else { return }
        context.coordinator.lastURL = fileURL
        context.coordinator.lastPack = resourcePack
        do {
            errorMessage = nil
            try controller.preparePreview(of: fileURL, resourcePack: resourcePack)
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    static func dismantleNSViewController(_ controller: SchematicMetalViewController, coordinator: Coordinator) {
        controller.cancelNativeLoad()
    }

    final class Coordinator {
        var lastURL: URL?
        var lastPack: NativeResourcePack?
    }
}
