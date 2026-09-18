import SwiftUI

struct SchematicWebView: NSViewControllerRepresentable {
    let fileURL: URL
    @Binding var errorMessage: String?

    func makeCoordinator() -> Coordinator {
        Coordinator()
    }

    func makeNSViewController(context: Context) -> SchematicWebViewController {
        SchematicWebViewController()
    }

    func updateNSViewController(
        _ controller: SchematicWebViewController,
        context: Context
    ) {
        context.coordinator.load(
            fileURL,
            in: controller,
            errorMessage: $errorMessage
        )
    }

    static func dismantleNSViewController(
        _: SchematicWebViewController,
        coordinator: Coordinator
    ) {
        coordinator.task?.cancel()
    }

    @MainActor
    final class Coordinator {
        fileprivate var task: Task<Void, Never>?
        private var lastURL: URL?
        private var loadGeneration = 0

        func load(
            _ url: URL,
            in controller: SchematicWebViewController,
            errorMessage: Binding<String?>
        ) {
            guard lastURL != url else {
                return
            }

            lastURL = url
            loadGeneration += 1
            let generation = loadGeneration
            task?.cancel()
            task = Task { @MainActor in
                do {
                    errorMessage.wrappedValue = nil
                    try await controller.preparePreview(of: url)
                } catch is CancellationError {
                    return
                } catch {
                    guard !Task.isCancelled,
                          generation == loadGeneration,
                          url == lastURL else {
                        return
                    }
                    errorMessage.wrappedValue = error.localizedDescription
                }
            }
        }
    }
}
