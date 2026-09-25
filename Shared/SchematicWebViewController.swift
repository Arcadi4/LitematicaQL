import AppKit
import OSLog
@preconcurrency import WebKit

@MainActor
final class SchematicWebViewController: NSViewController {
    private enum PageState {
        case failed(Error)
        case loading
        case ready
    }

    private static let messageHandlerName = "litematicaQL"
    private static let contentRuleIdentifier = "moe.arcadia.LitematicaQL.offline"
    private static let meshScheme = "lql-mesh"
    private static let logger = Logger(
        subsystem: "moe.arcadia.LitematicaQL",
        category: "Renderer"
    )
    private static let offlineContentRules = #"""
        [
          {"trigger":{"url-filter":"https?://.*"},"action":{"type":"block"}},
          {"trigger":{"url-filter":"wss?://.*"},"action":{"type":"block"}},
          {"trigger":{"url-filter":"ftp://.*"},"action":{"type":"block"}}
        ]
        """#

    /// Above this occupied-block count, render metadata before meshing so Quick Look
    /// presents promptly while a large native mesh is still running.
    private static let immediateRenderNoticeBlocks = 1_048_576

    private var pageState: PageState = .loading
    private var bootstrapTimeoutTask: Task<Void, Never>?
    private var configurationTask: Task<Void, Never>?
    private var readyWaiters: [UUID: CheckedContinuation<Void, any Error>] = [:]
    private var rendererDirectory: URL?
    private var webView: WKWebView?

    // Identifies the active load so superseded background meshes cannot publish stale geometry.
    private var loadGeneration = 0
    /// Retained so App teardown and Quick Look cancellation can interrupt native meshing.
    private var nativeLoad: Task<Void, Never>?
    private var nativeSession: NativeSchematicSession?

    deinit {
        let task = nativeLoad
        let session = nativeSession
        Task { @MainActor in
            task?.cancel()
            session?.cancel()
        }
    }

    func cancelNativeLoad() {
        loadGeneration += 1
        nativeLoad?.cancel()
        nativeSession?.cancel()
        nativeLoad = nil
        nativeSession = nil
        meshHandler.clear()
    }

    private let meshHandler = BatchResourceHandler()

    override func loadView() {
        let containerView = NSView()
        containerView.wantsLayer = true
        containerView.layer?.backgroundColor = NSColor(
            calibratedRed: 0.043,
            green: 0.063,
            blue: 0.086,
            alpha: 1
        ).cgColor
        view = containerView
        startBootstrapTimeout()

        configurationTask = Task { @MainActor [weak self] in
            do {
                guard let contentRuleList = try await WKContentRuleListStore.default().compileContentRuleList(
                    forIdentifier: Self.contentRuleIdentifier,
                    encodedContentRuleList: Self.offlineContentRules
                ) else {
                    throw PreviewInfrastructureError.offlineRulesUnavailable
                }
                try Task.checkCancellation()
                guard let self, case .loading = self.pageState else {
                    return
                }
                self.configureWebView(contentRuleList: contentRuleList)
            } catch is CancellationError {
                return
            } catch {
                self?.failPage(with: error)
            }
        }
    }

    private func configureWebView(contentRuleList: WKContentRuleList) {
        guard case .loading = pageState else {
            return
        }

        let configuration = WKWebViewConfiguration()
        configuration.websiteDataStore = .nonPersistent()
        configuration.defaultWebpagePreferences.allowsContentJavaScript = true
        configuration.preferences.javaScriptCanOpenWindowsAutomatically = false
        configuration.userContentController.add(contentRuleList)
        configuration.userContentController.add(
            WeakScriptMessageHandler(delegate: self),
            name: Self.messageHandlerName
        )
        configuration.setURLSchemeHandler(meshHandler, forURLScheme: Self.meshScheme)
        configuration.userContentController.addUserScript(
            WKUserScript(
                source: Self.javaScriptDiagnostics,
                injectionTime: .atDocumentStart,
                forMainFrameOnly: true
            )
        )

        let webView = WKWebView(frame: .zero, configuration: configuration)
        webView.translatesAutoresizingMaskIntoConstraints = false
        webView.navigationDelegate = self
        self.webView = webView
        view.addSubview(webView)

        NSLayoutConstraint.activate([
            webView.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            webView.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            webView.topAnchor.constraint(equalTo: view.topAnchor),
            webView.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        loadRendererPage()
    }

    /// Starts a preview without blocking the main actor on native decoding or meshing.
    ///
    /// Native content refusals are rendered in-page because Quick Look discards this
    /// window when `preparePreview` throws. File and infrastructure failures propagate.
    func preparePreview(of url: URL) async throws {
        _ = view
        try Task.checkCancellation()

        try await waitUntilReady()
        try Task.checkCancellation()

        let data = try await Task.detached(priority: .userInitiated) {
            try LitematicFile.readValidatedData(from: url)
        }.value
        try Task.checkCancellation()

        guard let rendererDirectory else {
            throw PreviewInfrastructureError.missingRenderer
        }
        let pack = try await Task.detached(priority: .userInitiated) {
            let data = try SchematicWebViewController.loadResourcePack(from: rendererDirectory)
            return try NativeResourcePack(data)
        }.value
        try Task.checkCancellation()

        loadGeneration += 1
        let generation = loadGeneration
        let displayName = url.lastPathComponent
        let handler = meshHandler
        let session = NativeSchematicSession()
        nativeLoad?.cancel()
        nativeSession?.cancel()
        nativeSession = session
        nativeLoad = Task.detached(priority: .userInitiated) { [weak self] in
            await withTaskCancellationHandler {
                await self?.runNativeLoad(
                    session: session,
                    data: data,
                    pack: pack,
                    displayName: displayName,
                    generation: generation,
                    handler: handler
                )
            } onCancel: {
                session.cancel()
            }
        }
    }

    private nonisolated func runNativeLoad(
        session: NativeSchematicSession,
        data: Data,
        pack: NativeResourcePack,
        displayName: String,
        generation: Int,
        handler: BatchResourceHandler
    ) async {
        defer {
            session.endMesh()
            handler.clear(generation: generation)
        }
        do {
            await presentProgress(phase: "decode", completed: 0, total: 1, bytes: data.count, triangles: 0)
            let facts = try session.decode(data)
            guard await isCurrentLoad(generation) else { return }
            await presentDecoded(
                displayName: displayName,
                facts: facts,
                warnings: facts.warnings,
                large: facts.blockCount > Self.immediateRenderNoticeBlocks
            )
            await presentProgress(phase: "decode", completed: 1, total: 1, bytes: data.count, triangles: 0)

            let mesh = try session.beginMesh(pack: pack)
            guard await isCurrentLoad(generation) else { return }
            handler.install(
                generation: generation,
                session: session,
                pack: pack,
                atlas: mesh.atlas,
                batchCount: mesh.batchCount
            ) { [weak self] (batch: NativeMeshBatch) in
                Task { @MainActor [weak self] in
                    await self?.presentProgress(
                        phase: "mesh",
                        completed: batch.index + 1,
                        total: mesh.batchCount,
                        bytes: batch.bytes,
                        triangles: batch.triangles
                    )
                }
            }
            await presentMesh(displayName: displayName, mesh: mesh)
        } catch is CancellationError {
            return
        } catch is NativeSchematicCancelled {
            return
        } catch let refusal as NativeSchematicRefusal {
            guard await isCurrentLoad(generation) else { return }
            await presentRefusal(refusal.message)
        } catch {
            guard await isCurrentLoad(generation) else { return }
            await presentRefusal("Something went wrong while reading this schematic.")
        }

    }

    private func isCurrentLoad(_ generation: Int) -> Bool {
        guard loadGeneration == generation else {
            return false
        }
        if case .ready = pageState {
            return true
        }
        return false
    }

    private func presentDecoded(
        displayName: String,
        facts: NativeSchematicFacts,
        warnings: [String],
        large: Bool
    ) async {
        guard let webView else { return }
        let dimensions = facts.contentDimensions
        _ = try? await webView.callAsyncJavaScript(
            "return window.litematicaQL.loadMeta(info);",
            arguments: [
                "info": [
                    "name": displayName,
                    "dimensions": "\(dimensions.0) × \(dimensions.1) × \(dimensions.2)",
                    "blockCount": facts.blockCount,
                    "blockEntityCount": facts.blockEntityCount,
                    "warnings": warnings,
                    "large": large,
                ]
            ],
            in: nil,
            contentWorld: .page
        )
    }

    private func presentMesh(displayName: String, mesh: NativeMeshStart) async {
        guard let webView else { return }
        _ = try? await webView.callAsyncJavaScript(
            "return window.litematicaQL.meshStart(info);",
            arguments: [
                "info": [
                    "name": displayName,
                    "atlasUrl": "lql-mesh://preview/atlas",
                    "atlasWidth": mesh.atlasWidth,
                    "atlasHeight": mesh.atlasHeight,
                    "batchCount": mesh.batchCount,
                ]
            ],
            in: nil,
            contentWorld: .page
        )
    }

    private func presentProgress(
        phase: String,
        completed: Int,
        total: Int,
        bytes: Int,
        triangles: Int
    ) async {
        guard let webView else { return }
        _ = try? await webView.callAsyncJavaScript(
            "return window.litematicaQL.progress(info);",
            arguments: [
                "info": [
                    "phase": phase,
                    "completed": completed,
                    "total": total,
                    "bytes": bytes,
                    "triangles": triangles,
                ]
            ],
            in: nil,
            contentWorld: .page
        )
    }

    private func presentRefusal(_ message: String) async {
        guard let webView else { return }
        _ = try? await webView.callAsyncJavaScript(
            "return window.litematicaQL.loadError(message);",
            arguments: ["message": message],
            in: nil,
            contentWorld: .page
        )
    }

    private nonisolated static func loadResourcePack(from directory: URL) throws -> Data {
        let resourcePackURL = directory.appendingPathComponent("pack.zip")
        let data = try Data(contentsOf: resourcePackURL, options: .mappedIfSafe)
        guard data.starts(with: [0x50, 0x4B]) else {
            throw PreviewInfrastructureError.invalidResourcePack
        }
        return data
    }

    private func loadRendererPage() {
        guard let rendererDirectory = Bundle(for: Self.self).url(
            forResource: "Renderer",
            withExtension: nil
        ) else {
            failPage(with: PreviewInfrastructureError.missingRenderer)
            return
        }

        let indexURL = rendererDirectory.appendingPathComponent("index.html")
        let resourcePackURL = rendererDirectory.appendingPathComponent("pack.zip")
        guard FileManager.default.fileExists(atPath: indexURL.path),
              FileManager.default.fileExists(atPath: resourcePackURL.path) else {
            failPage(with: PreviewInfrastructureError.missingRenderer)
            return
        }

        self.rendererDirectory = rendererDirectory.standardizedFileURL
        Self.logger.debug("Loading renderer from \(indexURL.path, privacy: .public)")
        webView?.loadFileURL(indexURL, allowingReadAccessTo: rendererDirectory)
    }

    private func waitUntilReady() async throws {
        let waiterID = UUID()
        try await withTaskCancellationHandler(
            operation: {
                try await waitForPageReady(waiterID: waiterID)
            },
            onCancel: {
                Task { @MainActor [weak self] in
                    self?.cancelReadyWaiter(waiterID)
                }
            }
        )
    }

    private func waitForPageReady(waiterID: UUID) async throws {
        try Task.checkCancellation()

        switch pageState {
        case .ready:
            return
        case let .failed(error):
            throw error
        case .loading:
            try await withCheckedThrowingContinuation {
                (continuation: CheckedContinuation<Void, any Error>) in
                if Task.isCancelled {
                    continuation.resume(throwing: CancellationError())
                } else {
                    readyWaiters[waiterID] = continuation
                }
            }
        }
        try Task.checkCancellation()
    }

    private func markPageReady() {
        guard case .loading = pageState else {
            return
        }

        bootstrapTimeoutTask?.cancel()
        bootstrapTimeoutTask = nil
        configurationTask?.cancel()
        configurationTask = nil
        pageState = .ready
        readyWaiters.values.forEach { $0.resume() }
        readyWaiters.removeAll()
    }

    private func failPage(with error: Error) {
        Self.logger.error("Renderer failed: \(error.localizedDescription, privacy: .public)")
        bootstrapTimeoutTask?.cancel()
        bootstrapTimeoutTask = nil
        configurationTask?.cancel()
        configurationTask = nil
        pageState = .failed(error)
        readyWaiters.values.forEach { $0.resume(throwing: error) }
        readyWaiters.removeAll()
    }

    private func cancelReadyWaiter(_ waiterID: UUID) {
        readyWaiters.removeValue(forKey: waiterID)?.resume(throwing: CancellationError())
    }

    private func startBootstrapTimeout() {
        bootstrapTimeoutTask?.cancel()
        bootstrapTimeoutTask = Task { @MainActor [weak self] in
            do {
                try await Task.sleep(nanoseconds: 20_000_000_000)
            } catch {
                return
            }
            self?.failPage(with: PreviewInfrastructureError.rendererTimedOut)
        }
    }

    private static let javaScriptDiagnostics = #"""
        window.addEventListener("error", (event) => {
          const detail = event.error?.stack || event.message || "Unknown JavaScript error";
          window.webkit?.messageHandlers?.litematicaQL?.postMessage({ type: "fatalError", detail });
        });
        window.addEventListener("unhandledrejection", (event) => {
          const reason = event.reason;
          const detail = reason?.stack || reason?.message || String(reason || "Unhandled promise rejection");
          window.webkit?.messageHandlers?.litematicaQL?.postMessage({ type: "fatalError", detail });
        });
        """#
}

extension SchematicWebViewController: WKNavigationDelegate {
    func webView(_ webView: WKWebView, didFinish _: WKNavigation!) {
        Task { @MainActor in
            do {
                let state = try await webView.callAsyncJavaScript(
                    """
                    return JSON.stringify({
                      bodyText: document.body?.innerText || "",
                      bridgeType: typeof window.litematicaQL,
                      readyState: document.readyState,
                      url: location.href,
                      webgl: Boolean(document.createElement("canvas").getContext("webgl2")),
                    });
                    """,
                    arguments: [:],
                    in: nil,
                    contentWorld: .page
                )
                Self.logger.debug("Renderer navigation finished: \(String(describing: state), privacy: .public)")
            } catch {
                Self.logger.error("Unable to inspect renderer page: \(error.localizedDescription, privacy: .public)")
            }
        }
    }

    func webView(
        _: WKWebView,
        didFail _: WKNavigation!,
        withError error: Error
    ) {
        failPage(with: error)
    }

    func webView(
        _: WKWebView,
        didFailProvisionalNavigation _: WKNavigation!,
        withError error: Error
    ) {
        failPage(with: error)
    }

    func webViewWebContentProcessDidTerminate(_: WKWebView) {
        failPage(with: PreviewInfrastructureError.webContentProcessTerminated)
    }

    func webView(
        _: WKWebView,
        decidePolicyFor navigationAction: WKNavigationAction
    ) async -> WKNavigationActionPolicy {
        guard let url = navigationAction.request.url,
              isRendererFileURL(url) else {
            if let url = navigationAction.request.url {
                Self.logger.notice("Blocked renderer navigation to \(url.absoluteString, privacy: .public)")
            }
            return .cancel
        }

        return .allow
    }

    private func isRendererFileURL(_ url: URL) -> Bool {
        guard url.isFileURL, let rendererDirectory else {
            return false
        }

        let rendererPath = rendererDirectory.path
        let candidatePath = url.standardizedFileURL.path
        return candidatePath == rendererPath || candidatePath.hasPrefix(rendererPath + "/")
    }
}

extension SchematicWebViewController: WKScriptMessageHandler {
    func userContentController(
        _: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        guard message.name == Self.messageHandlerName,
              let payload = message.body as? [String: Any],
              let type = payload["type"] as? String else {
            return
        }

        let detail = payload["detail"] as? String ?? ""
        Self.logger.debug("Renderer message: \(type, privacy: .public) \(detail, privacy: .public)")

        if type == "ready" {
            markPageReady()
        } else if type == "fatalError" {
            failPage(with: PreviewInfrastructureError.javaScript(detail))
        }
    }
}

/// Publishes the shared atlas once and lazily materializes ordered geometry
/// batches as the renderer requests them. WebKit's loader thread is the only
/// caller of `nextBatch`, so the native stream remains bounded to one payload.
final class BatchResourceHandler: NSObject, WKURLSchemeHandler, @unchecked Sendable {
    private struct Context {
        let generation: Int
        let session: NativeSchematicSession
        let pack: NativeResourcePack
        let atlas: Data
        let batchCount: Int
        let progress: (NativeMeshBatch) -> Void
    }

    private let lock = NSLock()
    nonisolated(unsafe) private var context: Context?

    nonisolated func install(
        generation: Int,
        session: NativeSchematicSession,
        pack: NativeResourcePack,
        atlas: Data,
        batchCount: Int,
        progress: @escaping (NativeMeshBatch) -> Void
    ) {
        lock.lock()
        context = Context(
            generation: generation,
            session: session,
            pack: pack,
            atlas: atlas,
            batchCount: batchCount,
            progress: progress
        )
        lock.unlock()
    }

    nonisolated func clear(generation: Int? = nil) {
        lock.lock()
        if generation == nil || context?.generation == generation {
            context = nil
        }
        lock.unlock()
    }

    nonisolated func webView(_: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url,
              url.scheme == "lql-mesh",
              url.host == "preview" else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }
        lock.lock()
        let context = self.context
        lock.unlock()
        guard let context else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }

        let data: Data
        let contentType: String
        if url.path == "/atlas" {
            data = context.atlas
            contentType = "image/png"
        } else {
            let components = url.path.split(separator: "/")
            guard components.count == 2,
                  components[0] == "batch",
                  let index = Int(components[1]),
                  index >= 0,
                  index < context.batchCount else {
                task.didFailWithError(URLError(.fileDoesNotExist))
                return
            }
            do {
                guard let batch = try context.session.nextBatch(index: index) else {
                    task.didFailWithError(URLError(.fileDoesNotExist))
                    return
                }
                data = batch.payload
                contentType = "application/vnd.litematicaql.batch"
                context.progress(batch)
            } catch {
                task.didFailWithError(error as NSError)
                return
            }
        }

        let response = HTTPURLResponse(
            url: url,
            statusCode: 200,
            httpVersion: "HTTP/1.1",
            headerFields: [
                "Content-Type": contentType,
                "Content-Length": String(data.count),
                "Access-Control-Allow-Origin": "*",
                "Cache-Control": "no-store",
            ]
        )
        if let response { task.didReceive(response) }
        task.didReceive(data)
        task.didFinish()
    }

    nonisolated func webView(_: WKWebView, stop _: WKURLSchemeTask) {
    }
}

private final class WeakScriptMessageHandler: NSObject, WKScriptMessageHandler {
    private weak var delegate: WKScriptMessageHandler?

    init(delegate: WKScriptMessageHandler) {
        self.delegate = delegate
    }

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) {
        delegate?.userContentController(userContentController, didReceive: message)
    }
}

private enum PreviewInfrastructureError: LocalizedError {
    case invalidResourcePack
    case javaScript(String)
    case missingRenderer
    case offlineRulesUnavailable
    case rendererTimedOut
    case webContentProcessTerminated

    var errorDescription: String? {
        switch self {
        case .invalidResourcePack:
            return "The bundled block resources are invalid. Rebuild the renderer assets and the app."
        case let .javaScript(message):
            return "The renderer failed: \(message)"
        case .missingRenderer:
            return "The bundled Litematica renderer is missing. Rebuild the renderer assets and the app."
        case .offlineRulesUnavailable:
            return "The offline renderer security policy could not be created."
        case .rendererTimedOut:
            return "The renderer did not become ready within 20 seconds."
        case .webContentProcessTerminated:
            return "The WebKit renderer process stopped unexpectedly."
        }
    }
}
