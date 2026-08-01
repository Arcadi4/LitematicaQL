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
    private static let resourcePackHandlerName = "litematicaQLResourcePack"
    private static let contentRuleIdentifier = "moe.arcadia.LitematicaQL.offline"
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

    private var pageState: PageState = .loading
    private var bootstrapTimeoutTask: Task<Void, Never>?
    private var configurationTask: Task<Void, Never>?
    private var readyWaiters: [UUID: CheckedContinuation<Void, any Error>] = [:]
    private var rendererDirectory: URL?
    private var webView: WKWebView?

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
                configureWebView(contentRuleList: contentRuleList)
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
        configuration.userContentController.addScriptMessageHandler(
            WeakReplyScriptMessageHandler(delegate: self),
            contentWorld: .page,
            name: Self.resourcePackHandlerName
        )
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

    func preparePreview(of url: URL) async throws {
        _ = view
        try Task.checkCancellation()

        let dataTask = Task.detached(priority: .userInitiated) {
            try LitematicFile.readValidatedData(from: url)
        }

        do {
            try await waitUntilReady()
            try Task.checkCancellation()
            let data = try await dataTask.value
            try Task.checkCancellation()
            let encodedData = await Task.detached(priority: .userInitiated) {
                data.base64EncodedString()
            }.value
            try Task.checkCancellation()

            guard let webView else {
                throw PreviewInfrastructureError.missingRenderer
            }

            _ = try await webView.callAsyncJavaScript(
                "return window.litematicaQL.loadSchematic(name, encodedData);",
                arguments: [
                    "name": url.deletingPathExtension().lastPathComponent,
                    "encodedData": encodedData,
                ],
                in: nil,
                contentWorld: .page
            )
            try Task.checkCancellation()
        } catch {
            dataTask.cancel()
            throw error
        }
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

extension SchematicWebViewController: WKScriptMessageHandlerWithReply {
    func userContentController(
        _: WKUserContentController,
        didReceive message: WKScriptMessage
    ) async -> (Any?, String?) {
        guard message.name == Self.resourcePackHandlerName,
              let payload = message.body as? [String: Any],
              payload["type"] as? String == "resourcePack",
              let rendererDirectory else {
            return (nil, PreviewInfrastructureError.invalidResourcePackRequest.localizedDescription)
        }

        let resourcePackURL = rendererDirectory.appendingPathComponent("pack.zip")
        do {
            let encodedData = try await Task.detached(priority: .userInitiated) {
                let data = try Data(contentsOf: resourcePackURL, options: .mappedIfSafe)
                guard data.starts(with: [0x50, 0x4B]) else {
                    throw PreviewInfrastructureError.invalidResourcePack
                }
                return data.base64EncodedString()
            }.value
            return (encodedData, nil)
        } catch {
            Self.logger.error("Unable to bridge resource pack: \(error.localizedDescription, privacy: .public)")
            return (nil, error.localizedDescription)
        }
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

private final class WeakReplyScriptMessageHandler: NSObject, WKScriptMessageHandlerWithReply {
    private weak var delegate: WKScriptMessageHandlerWithReply?

    init(delegate: WKScriptMessageHandlerWithReply) {
        self.delegate = delegate
    }

    func userContentController(
        _ userContentController: WKUserContentController,
        didReceive message: WKScriptMessage
    ) async -> (Any?, String?) {
        guard let delegate else {
            return (nil, "The native resource-pack bridge is unavailable.")
        }

        return await delegate.userContentController(
            userContentController,
            didReceive: message
        )
    }
}

private enum PreviewInfrastructureError: LocalizedError {
    case invalidResourcePack
    case invalidResourcePackRequest
    case javaScript(String)
    case missingRenderer
    case offlineRulesUnavailable
    case rendererTimedOut
    case webContentProcessTerminated

    var errorDescription: String? {
        switch self {
        case .invalidResourcePack:
            return "The bundled block resources are invalid. Rebuild the renderer assets and the app."
        case .invalidResourcePackRequest:
            return "The renderer made an invalid resource-pack request."
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
