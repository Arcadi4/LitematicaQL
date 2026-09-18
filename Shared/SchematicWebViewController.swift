// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

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
    private static let glbURL = URL(string: "lql-glb://preview/mesh.glb")
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

    // Builds past this many non-empty blocks mesh for a noticeable while, so
    // their window presents immediately with a render notice instead of
    // holding back until the first frame exists.
    private static let immediateRenderNoticeBlocks = 1_048_576

    private var pageState: PageState = .loading
    private var bootstrapTimeoutTask: Task<Void, Never>?
    private var configurationTask: Task<Void, Never>?
    private var readyWaiters: [UUID: CheckedContinuation<Void, any Error>] = [:]
    private var rendererDirectory: URL?
    private var webView: WKWebView?

    // Increments on every load so a superseded background mesh abandons its
    // result instead of painting it over the newer load.
    private var loadGeneration = 0

    private let glbHandler = GLBResourceHandler()

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
        // The mesh crosses to the page as raw bytes over a custom scheme,
        // never as a base64 string.
        configuration.setURLSchemeHandler(glbHandler, forURLScheme: "lql-glb")
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

    // Presents the preview for a schematic file.
    //
    // Decoding and meshing run natively off the main thread. Content refusals
    // (unreadable files, oversized builds, exhausted meshes) resolve inside
    // the page as panels, because Quick Look discards this window entirely
    // when the call throws. Only infrastructure failures throw.
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
            try SchematicWebViewController.loadResourcePack(from: rendererDirectory)
        }.value
        try Task.checkCancellation()

        loadGeneration += 1
        let generation = loadGeneration
        let displayName = url.lastPathComponent
        let handler = glbHandler

        Task.detached(priority: .userInitiated) {
            await self.runNativeLoad(
                data: data,
                pack: pack,
                displayName: displayName,
                generation: generation,
                handler: handler
            )
        }
    }

    // Drives decode, metadata, mesh, and display for one load.
    //
    // The whole flow runs on one background task because the native handle
    // must stay on the thread that opened it.
    private nonisolated func runNativeLoad(
        data: Data,
        pack: Data,
        displayName: String,
        generation: Int,
        handler: GLBResourceHandler
    ) async {
        let session = NativeSchematicSession()
        do {
            let facts = try session.decode(data)
            guard await isCurrentLoad(generation) else { return }
            await presentDecoded(
                displayName: displayName,
                facts: facts,
                large: facts.blockCount > Self.immediateRenderNoticeBlocks
            )

            let mesh = try session.mesh(pack: pack)
            handler.store(mesh.glb)
            guard await isCurrentLoad(generation) else {
                handler.clear()
                return
            }
            await presentMesh(displayName: displayName, triangles: mesh.triangleCount)
        } catch let refusal as NativeSchematicRefusal {
            guard await isCurrentLoad(generation) else { return }
            await presentRefusal(refusal.message)
        } catch {
            guard await isCurrentLoad(generation) else { return }
            await presentRefusal(
                "Something went wrong while reading this schematic."
            )
        }
    }

    // Whether `generation` is still the load the window should show.
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
                    "large": large,
                ]
            ],
            in: nil,
            contentWorld: .page
        )
    }

    private func presentMesh(displayName: String, triangles: Int) async {
        guard let webView else { return }
        _ = try? await webView.callAsyncJavaScript(
            "return window.litematicaQL.meshReady(info);",
            arguments: [
                "info": [
                    "name": displayName,
                    "triangles": triangles,
                    "url": Self.glbURL?.absoluteString ?? "",
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

// Serves the meshed GLB to the page over the `lql-glb` custom scheme.
//
// The mesh lands here from a background task and is read back by WebKit's
// loader thread, so access goes through a lock; the class is its own
// synchronization.
final class GLBResourceHandler: NSObject, WKURLSchemeHandler, @unchecked Sendable {
    private let lock = NSLock()
    private var glb: Data?

    // Publishes the mesh the page should fetch, replacing any stale one.
    nonisolated func store(_ data: Data) {
        lock.lock()
        defer { lock.unlock() }
        glb = data
    }

    // Drops the mesh when a superseded load must not serve stale geometry.
    nonisolated func clear() {
        lock.lock()
        defer { lock.unlock() }
        glb = nil
    }

    func webView(_: WKWebView, start task: WKURLSchemeTask) {
        guard let url = task.request.url,
              url.scheme == "lql-glb",
              url.host == "preview",
              url.path == "/mesh.glb" else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }

        lock.lock()
        let data = glb
        lock.unlock()

        guard let data else {
            task.didFailWithError(URLError(.fileDoesNotExist))
            return
        }

        // The page origin is `file://`, so the response must opt into CORS or
        // the fetch is refused.
        let response = HTTPURLResponse(
            url: url,
            statusCode: 200,
            httpVersion: "HTTP/1.1",
            headerFields: [
                "Content-Type": "model/gltf-binary",
                "Content-Length": String(data.count),
                "Access-Control-Allow-Origin": "*",
                "Cache-Control": "no-store",
            ]
        )
        if let response {
            task.didReceive(response)
        }
        task.didReceive(data)
        task.didFinish()
    }

    func webView(_: WKWebView, stop _: WKURLSchemeTask) {
        // Delivery is a single synchronous burst; nothing to unwind.
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
