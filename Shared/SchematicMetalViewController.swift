import AppKit
import MetalKit
import simd

/// The app and Quick Look share the same native view and bounded loader.
@MainActor
final class SchematicMetalViewController: NSViewController {
    private var renderer: SchematicMetalRenderer?
    private var nativeLoad: Task<Void, Never>?
    private var nativeSession: NativeSchematicSession?
    private var generation = 0
    /// File and pack of the current preview. An override retry after a limit
    /// refusal re-reads this exact pair.
    private var retryURL: URL?
    private var retryPack: NativeResourcePack?
    /// A texture-pack problem that the host names while it renders with the
    /// bundled base. The retry keeps the same notice.
    private var packWarning: String?
    private var retryPackWarning: String?
    private let status = NSTextField(wrappingLabelWithString: "")
    private let details = NSTextField(wrappingLabelWithString: "")
    private let progress = NSProgressIndicator()
    private let reset = NSButton(title: "Reset View", target: nil, action: nil)

    override func loadView() {
        view = NSView()
        view.wantsLayer = true
        let metal = SchematicMetalView(frame: .zero, device: MTLCreateSystemDefaultDevice())
        metal.translatesAutoresizingMaskIntoConstraints = false
        metal.isPaused = true
        metal.enableSetNeedsDisplay = true
        metal.framebufferOnly = true
        metal.colorPixelFormat = .bgra8Unorm_srgb
        metal.depthStencilPixelFormat = .depth32Float
        metal.sampleCount = 1
        metal.setAccessibilityLabel("Minecraft schematic")
        metal.setAccessibilityHelp("Drag to orbit, scroll to zoom, right-drag to pan. Press R to reset the view.")
        view.addSubview(metal)
        NSLayoutConstraint.activate([
            metal.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            metal.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            metal.topAnchor.constraint(equalTo: view.topAnchor),
            metal.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])

        let panel = NSVisualEffectView()
        panel.material = .hudWindow
        panel.blendingMode = .withinWindow
        panel.state = .active
        panel.wantsLayer = true
        panel.layer?.cornerRadius = 8
        panel.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(panel)
        status.font = .systemFont(ofSize: 12, weight: .medium)
        details.font = .systemFont(ofSize: 11)
        details.textColor = .secondaryLabelColor
        progress.style = .bar
        progress.isIndeterminate = false
        progress.minValue = 0
        progress.maxValue = 1
        progress.isHidden = true
        let stack = NSStackView(views: [status, details, progress])
        stack.orientation = .vertical
        stack.alignment = .leading
        stack.spacing = 5
        stack.translatesAutoresizingMaskIntoConstraints = false
        panel.addSubview(stack)
        reset.target = self
        reset.action = #selector(resetCamera)
        reset.bezelStyle = .rounded
        reset.translatesAutoresizingMaskIntoConstraints = false
        reset.toolTip = "Fit the schematic in the view (R)"
        view.addSubview(reset)
        NSLayoutConstraint.activate([
            panel.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 14),
            panel.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -14),
            panel.widthAnchor.constraint(lessThanOrEqualTo: view.widthAnchor, multiplier: 0.7),
            stack.leadingAnchor.constraint(equalTo: panel.leadingAnchor, constant: 12),
            stack.trailingAnchor.constraint(equalTo: panel.trailingAnchor, constant: -12),
            stack.topAnchor.constraint(equalTo: panel.topAnchor, constant: 10),
            stack.bottomAnchor.constraint(equalTo: panel.bottomAnchor, constant: -10),
            progress.widthAnchor.constraint(equalToConstant: 180),
            reset.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -14),
            reset.bottomAnchor.constraint(equalTo: view.bottomAnchor, constant: -14),
        ])
        do {
            guard let device = metal.device else {
                throw NativeSchematicRefusal(message: "This Mac does not have a Metal graphics device.")
            }
            let renderer = try SchematicMetalRenderer(view: metal, device: device)
            renderer.onError = { [weak self] message in self?.showFailure(message) }
            self.renderer = renderer
            metal.renderer = renderer
        } catch { showFailure(error.localizedDescription) }
        metal.viewDidChangeEffectiveAppearance()
    }

    deinit {
        nativeLoad?.cancel()
        nativeSession?.cancel()
    }

    override func viewDidDisappear() {
        super.viewDidDisappear()
        cancelNativeLoad()
    }

    func cancelNativeLoad() {
        generation += 1
        nativeSession?.cancel()
        nativeLoad?.cancel()
        nativeSession = nil
        nativeLoad = nil
    }

    func preparePreview(of url: URL, resourcePack: NativeResourcePack? = nil,
                        texturePackWarning: String? = nil,
                        overridesPreviewLimits: Bool = false) throws {
        _ = view
        try Task.checkCancellation()
        guard LitematicFile.supports(url) else { throw LitematicFileError.unsupportedExtension }
        guard let renderer else { return }
        cancelNativeLoad()
        retryURL = url
        retryPack = resourcePack
        retryPackWarning = texturePackWarning
        packWarning = texturePackWarning
        renderer.clear()
        let current = generation
        let session = NativeSchematicSession(
            // The user's render-anyway choice lifts both stages' preview quotas
            // up front, so an oversized schematic renders without the alert.
            overridesPreviewLimits: overridesPreviewLimits
                || RenderLimits.bypassesPreviewLimits()
        )
        nativeSession = session
        let uploader = MetalMeshUploader(device: renderer.device, queue: renderer.queue)
        // The pack parse and the schematic read run in parallel, so the pack
        // textures are ready when the atlas arrives. The parse does not extend
        // the load's critical path.
        let requestedPack = resourcePack
        status.stringValue = "Reading schematic…"
        details.stringValue = ""
        details.isHidden = true
        progress.isHidden = false
        progress.doubleValue = 0
        // The load starts on a detached task, so preparePreview returns control
        // to Quick Look at once and it can show progressive geometry. Decoding
        // and stream progress do not depend on drawing the view.
        nativeLoad = Task.detached(priority: .userInitiated) { [weak self] in
            let packReady = Task { () throws -> NativeResourcePack in
                try requestedPack ?? NativeResourcePack.bundled.get()
            }
            await withTaskCancellationHandler {
                do {
                    let facts = try autoreleasepool {
                        let data = try LitematicFile.readValidatedData(from: url)
                        return try session.decode(data)
                    }
                    try Task.checkCancellation()
                    // Pin the camera before any geometry exists, so streamed
                    // batches cannot move the view during the load.
                    await self?.pinFrame(current, facts: facts)
                    await self?.update(current, message: "Preparing block textures…", fraction: 0)
                    let pack = try await packReady.value
                    let batchCount = try autoreleasepool {
                        let start = try session.beginMesh(pack: pack)
                        try uploader.uploadAtlas(start)
                        return start.batchCount
                    }
                    defer { session.endMesh() }
                    // The atlas is on the GPU. Everything after this point is
                    // geometry.
                    await self?.update(current, message: "Building geometry…", fraction: 0)
                    var lastUpdate = ContinuousClock.now
                    for index in 0..<batchCount {
                        try Task.checkCancellation()
                        let chunk = try autoreleasepool { () -> MetalMeshChunk? in
                            guard let batch = try session.nextBatch(index: index) else { return nil }
                            return try uploader.upload(batch)
                        }
                        let now = ContinuousClock.now
                        let repaint = index == 0 || now - lastUpdate >= .milliseconds(50)
                        if repaint { lastUpdate = now }
                        await self?.receive(chunk, generation: current, completed: index + 1,
                                            total: batchCount, repaint: repaint)
                    }
                    try uploader.finish()
                    try Task.checkCancellation()
                    await self?.finish(current, facts: facts)
                } catch is CancellationError {
                    // A replacement preview owns its UI and resources, so this
                    // cancellation needs no cleanup.
                } catch let refusal as NativeSchematicRefusal {
                    await self?.fail(current, message: refusal.message)
                    await self?.offerOverride(refusal)
                } catch {
                    await self?.fail(current, message: error.localizedDescription)
                }
            } onCancel: { session.cancel() }
        }
    }

    private func update(_ generation: Int, message: String, fraction: Double) {
        guard generation == self.generation else { return }
        status.stringValue = message
        progress.doubleValue = fraction
    }

    private func receive(_ chunk: MetalMeshChunk?, generation: Int, completed: Int, total: Int, repaint: Bool) {
        guard generation == self.generation else { return }
        if let chunk { renderer?.append(chunk) }
        if repaint {
            update(generation, message: "Building geometry · \(completed) / \(total)",
                   fraction: Double(completed) / Double(max(total, 1)))
            renderer?.requestDraw()
        }
    }

    private func pinFrame(_ generation: Int, facts: NativeSchematicFacts) {
        guard generation == self.generation else { return }
        let (x, y, z) = facts.contentDimensions
        guard x > 0, y > 0, z > 0 else { return }
        let (ox, oy, oz) = facts.contentOrigin
        renderer?.setContentBounds(min: SIMD3(ox, oy, oz), size: SIMD3(x, y, z))
    }

    private func finish(_ generation: Int, facts: NativeSchematicFacts) {
        guard generation == self.generation else { return }
        renderer?.settleFraming()
        let size = renderer?.dimensions ?? .zero
        status.stringValue = "\(facts.blockCount.formatted()) blocks · \(size.x) × \(size.y) × \(size.z)"
        let notices = ([packWarning].compactMap { $0 } + facts.warnings)
        details.stringValue = notices.joined(separator: "\n")
        details.isHidden = notices.isEmpty
        progress.isHidden = true
        nativeSession = nil
        nativeLoad = nil
        renderer?.requestDraw()
    }

    private func fail(_ generation: Int, message: String) {
        guard generation == self.generation else { return }
        renderer?.clear()
        nativeSession = nil
        nativeLoad = nil
        showFailure(message)
    }

    /// Offers to render past the preview quota that the file exceeded. Both
    /// hosts show this sheet, so Quick Look gets the same offer as the app
    /// window. A host whose user opted out of warnings renders past the quotas
    /// from the start and never shows this sheet.
    private func offerOverride(_ refusal: NativeSchematicRefusal) {
        guard refusal.isPreviewLimit, !RenderLimits.bypassesPreviewLimits(),
              let url = retryURL, let window = view.window else { return }
        let alert = NSAlert()
        alert.messageText = "This schematic is difficult to render"
        alert.informativeText = """
            \(refusal.message)

            Rendering it anyway may take forever or exhaust your memory.
            """
        alert.alertStyle = .warning
        alert.addButton(withTitle: "Render Anyway")
        alert.addButton(withTitle: "Cancel")
        alert.beginSheetModal(for: window) { response in
            guard response == .alertFirstButtonReturn else { return }
            do {
                try self.preparePreview(of: url, resourcePack: self.retryPack,
                                        texturePackWarning: self.retryPackWarning,
                                        overridesPreviewLimits: true)
            } catch {
                self.showFailure(error.localizedDescription)
            }
        }
    }

    private func showFailure(_ message: String) {
        status.stringValue = "Unable to preview schematic"
        details.stringValue = ([packWarning].compactMap { $0 } + [message]).joined(separator: "\n")
        details.isHidden = false
        progress.isHidden = true
    }

    @objc private func resetCamera() { renderer?.resetCamera() }
}
