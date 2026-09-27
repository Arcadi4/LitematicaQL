import AppKit
import MetalKit
import simd

@MainActor
final class SchematicMetalRenderer: NSObject, MTKViewDelegate {
    let device: MTLDevice
    let queue: MTLCommandQueue
    private weak var view: MTKView?
    private let pipelines: [MTLRenderPipelineState]
    private let depthWrite: MTLDepthStencilState
    private let depthRead: MTLDepthStencilState
    private let clampSampler: MTLSamplerState
    private let repeatSampler: MTLSamplerState
    private let frames = DispatchSemaphore(value: 2)
    private var redrawPending = false
    private var chunks: [MetalMeshChunk] = []
    private var low = SIMD3<Float>(repeating: .infinity)
    private var high = SIMD3<Float>(repeating: -.infinity)
    // The camera frames this box, not the geometry bounds. It is seeded from
    // the decoded schematic's occupied-block bounds, so streaming batches never
    // move the camera; the box only grows to admit geometry that reaches past
    // those bounds, and the camera is re-fit once when loading settles.
    private var frameLow = SIMD3<Float>(repeating: .infinity)
    private var frameHigh = SIMD3<Float>(repeating: -.infinity)
    private var framingPinned = false
    private var target = SIMD3<Float>(repeating: 0)
    private var distance: Float = 10
    private var yaw: Float = .pi / 4
    private var pitch: Float = .pi / 6
    private var cameraMoved = false
    var onError: ((String) -> Void)?

    init(view: MTKView, device: MTLDevice) throws {
        self.device = device
        self.view = view
        guard let queue = device.makeCommandQueue() else {
            throw NativeSchematicRefusal(message: "Metal could not create a graphics queue.")
        }
        self.queue = queue
        queue.label = "Schematic rendering and uploads"
        let library = try device.makeDefaultLibrary(bundle: Bundle.main)
        let vertices = MTLVertexDescriptor()
        for (index, format, offset) in [(0, MTLVertexFormat.ushort4Normalized, 0),
                                        (1, .float2, 8), (2, .char4Normalized, 16),
                                        (3, .uchar4Normalized, 20)] {
            vertices.attributes[index].format = format
            vertices.attributes[index].offset = offset
            vertices.attributes[index].bufferIndex = 0
        }
        vertices.layouts[0].stride = 24
        var pipelines: [MTLRenderPipelineState] = []
        for (index, fragment) in ["schematicOpaque", "schematicCutout", "schematicBlend"].enumerated() {
            let descriptor = MTLRenderPipelineDescriptor()
            descriptor.label = fragment
            descriptor.vertexFunction = library.makeFunction(name: "schematicVertex")
            descriptor.fragmentFunction = library.makeFunction(name: fragment)
            descriptor.vertexDescriptor = vertices
            descriptor.colorAttachments[0].pixelFormat = view.colorPixelFormat
            descriptor.depthAttachmentPixelFormat = view.depthStencilPixelFormat
            if index == 2 {
                let color = descriptor.colorAttachments[0]!
                color.isBlendingEnabled = true
                color.sourceRGBBlendFactor = .sourceAlpha
                color.destinationRGBBlendFactor = .oneMinusSourceAlpha
                color.sourceAlphaBlendFactor = .one
                color.destinationAlphaBlendFactor = .oneMinusSourceAlpha
            }
            pipelines.append(try device.makeRenderPipelineState(descriptor: descriptor))
        }
        self.pipelines = pipelines
        let depth = MTLDepthStencilDescriptor()
        depth.depthCompareFunction = .lessEqual
        depth.isDepthWriteEnabled = true
        guard let depthWrite = device.makeDepthStencilState(descriptor: depth) else {
            throw NativeSchematicRefusal(message: "Metal could not create a depth state.")
        }
        self.depthWrite = depthWrite
        depth.isDepthWriteEnabled = false
        self.depthRead = device.makeDepthStencilState(descriptor: depth)!
        let sampler = MTLSamplerDescriptor()
        sampler.minFilter = .nearest
        sampler.magFilter = .nearest
        sampler.mipFilter = .notMipmapped
        sampler.sAddressMode = .clampToEdge
        sampler.tAddressMode = .clampToEdge
        self.clampSampler = device.makeSamplerState(descriptor: sampler)!
        sampler.sAddressMode = .repeat
        sampler.tAddressMode = .repeat
        sampler.mipFilter = .linear
        sampler.maxAnisotropy = 4
        self.repeatSampler = device.makeSamplerState(descriptor: sampler)!
        super.init()
        view.delegate = self
    }

    func clear() {
        chunks.removeAll(keepingCapacity: false)
        low = SIMD3(repeating: .infinity)
        high = SIMD3(repeating: -.infinity)
        frameLow = SIMD3(repeating: .infinity)
        frameHigh = SIMD3(repeating: -.infinity)
        framingPinned = false
        cameraMoved = false
        yaw = .pi / 4
        pitch = .pi / 6
        requestDraw()
    }

    /// Pins the camera frame to the schematic's occupied-block bounds, which
    /// are known before meshing starts. Without this the frame would follow the
    /// growing geometry bounds and the view would drift as chunks stream in.
    func setContentBounds(min: SIMD3<Int>, size: SIMD3<Int>) {
        let origin = SIMD3<Float>(Float(min.x), Float(min.y), Float(min.z))
        let extent = SIMD3<Float>(Float(size.x), Float(size.y), Float(size.z))
        // Geometry is authored in block-center coordinates, so a block at `p`
        // spans [p - 0.5, p + 0.5]. Pad the block box to keep that margin.
        frameLow = simd_min(frameLow, origin - 0.5)
        frameHigh = simd_max(frameHigh, origin + extent - 0.5)
        framingPinned = true
        if !cameraMoved { fit() }
        requestDraw()
    }

    /// Re-frames once all geometry has arrived, so entities rendered outside
    /// the block bounds still sit inside the view.
    func settleFraming() {
        guard framingPinned, !cameraMoved else { return }
        fit()
        requestDraw()
    }

    func append(_ chunk: MetalMeshChunk) {
        chunks.append(chunk)
        low = simd_min(low, chunk.origin)
        high = simd_max(high, chunk.origin + chunk.extent)
        // Geometry may reach past the block bounds. Admit it to the frame, but
        // hold the camera still until loading settles so the view never drifts.
        frameLow = simd_min(frameLow, chunk.origin)
        frameHigh = simd_max(frameHigh, chunk.origin + chunk.extent)
        if !framingPinned, !cameraMoved { fit() }
    }

    var dimensions: SIMD3<Int> {
        guard !chunks.isEmpty else { return .zero }
        let size = high - low
        return SIMD3(Int(ceil(size.x)), Int(ceil(size.y)), Int(ceil(size.z)))
    }

    func resetCamera() {
        cameraMoved = false
        yaw = .pi / 4
        pitch = .pi / 6
        fit()
        requestDraw()
    }

    private var radius: Float {
        guard frameLow.x.isFinite else { return 1 }
        return max(simd_length(frameHigh - frameLow) * 0.5, 0.5)
    }
    private var eye: SIMD3<Float> {
        target + SIMD3(sin(yaw) * cos(pitch), sin(pitch), cos(yaw) * cos(pitch)) * distance
    }

    private func fit() {
        guard frameLow.x.isFinite else { return }
        target = (frameLow + frameHigh) * 0.5
        let size = view?.drawableSize ?? CGSize(width: 1, height: 1)
        let aspect = Float(max(size.width, 1) / max(size.height, 1))
        let angle = min(Float.pi / 8, atan(tan(Float.pi / 8) * aspect))
        distance = radius / sin(angle) * 1.08
    }

    func orbit(dx: Float, dy: Float) {
        cameraMoved = true
        yaw -= dx * 0.008
        pitch = min(max(pitch + dy * 0.008, -.pi / 2 + 0.01), .pi / 2 - 0.01)
        requestDraw()
    }

    func pan(dx: Float, dy: Float) {
        cameraMoved = true
        let backwards = simd_normalize(eye - target)
        let right = simd_normalize(simd_cross(SIMD3<Float>(0, 1, 0), backwards))
        let up = simd_cross(backwards, right)
        let height = max(Float(view?.bounds.height ?? 1), 1)
        target += (-right * dx + up * dy) * (2 * distance * tan(.pi / 8) / height)
        requestDraw()
    }

    func zoom(_ amount: Float) {
        cameraMoved = true
        distance = min(max(distance * exp(amount), 0.05), max(radius * 100, 10))
        requestDraw()
    }

    func requestDraw() {
        view?.needsDisplay = true
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {
        if !cameraMoved { fit() }
        requestDraw()
    }

    func draw(in view: MTKView) {
        guard view.drawableSize.width > 0, view.drawableSize.height > 0 else { return }
        guard frames.wait(timeout: .now()) == .success else {
            redrawPending = true
            return
        }
        guard let pass = view.currentRenderPassDescriptor, let drawable = view.currentDrawable,
              let command = queue.makeCommandBuffer() else {
            frames.signal()
            return
        }
        pass.depthAttachment.storeAction = .dontCare
        guard let encoder = command.makeRenderCommandEncoder(descriptor: pass) else {
            frames.signal()
            return
        }
        redrawPending = false
        command.label = "Draw schematic"
        var projection = viewProjection(size: view.drawableSize)
        encoder.setVertexBytes(&projection, length: MemoryLayout<simd_float4x4>.stride, index: 1)
        encoder.setFrontFacing(.counterClockwise)
        // Cull chunk AABBs before encoding any material draws. Sort once near
        // to far for opaque early depth rejection, then reverse for blending.
        let planes = frustumPlanes(projection)
        let eye = self.eye
        let visible = chunks.filter { chunk in
            let center = chunk.center
            let half = chunk.extent * 0.5
            return planes.allSatisfy { plane in
                let n = SIMD3(plane.x, plane.y, plane.z)
                return simd_dot(n, center) + plane.w + simd_dot(simd_abs(n), half) >= 0
            }
        }.sorted { simd_length_squared($0.center - eye) < simd_length_squared($1.center - eye) }
        for alpha in 0...2 {
            encoder.setRenderPipelineState(pipelines[alpha])
            encoder.setDepthStencilState(alpha == 2 ? depthRead : depthWrite)
            encoder.setCullMode(alpha == 0 ? .back : .none)
            if alpha == 2 {
                for chunk in visible.reversed() { encode(chunk, alpha: alpha, with: encoder) }
            } else {
                for chunk in visible { encode(chunk, alpha: alpha, with: encoder) }
            }
        }
        encoder.endEncoding()
        command.present(drawable)
        command.addCompletedHandler { [weak self, frames] command in
            frames.signal()
            let error = command.error?.localizedDescription
            Task { @MainActor [weak self] in
                if let error { self?.onError?(error) }
                if self?.redrawPending == true { self?.requestDraw() }
            }
        }
        command.commit()
    }

    private func encode(_ chunk: MetalMeshChunk, alpha: Int, with encoder: MTLRenderCommandEncoder) {
        var transform = (SIMD4(chunk.origin, 0), SIMD4(chunk.extent, 0))
        encoder.setVertexBytes(&transform, length: 32, index: 2)
        for part in chunk.parts where part.alphaMode == alpha {
            encoder.setVertexBuffer(chunk.buffer, offset: part.vertexOffset, index: 0)
            encoder.setFragmentTexture(part.texture, index: 0)
            encoder.setFragmentSamplerState(part.repeats ? repeatSampler : clampSampler, index: 0)
            encoder.drawIndexedPrimitives(type: .triangle, indexCount: part.indexCount,
                                          indexType: part.indexType, indexBuffer: chunk.buffer,
                                          indexBufferOffset: part.indexOffset)
        }
    }

    private func viewProjection(size: CGSize) -> simd_float4x4 {
        let eye = self.eye
        let z = simd_normalize(eye - target)
        let x = simd_normalize(simd_cross(SIMD3<Float>(0, 1, 0), z))
        let y = simd_cross(z, x)
        let view = simd_float4x4(columns: (
            SIMD4(x.x, y.x, z.x, 0), SIMD4(x.y, y.y, z.y, 0), SIMD4(x.z, y.z, z.z, 0),
            SIMD4(-simd_dot(x, eye), -simd_dot(y, eye), -simd_dot(z, eye), 1)
        ))
        // Measure the scene against the same pinned frame the camera uses, so
        // depth precision does not shift as batches stream in.
        let sceneCenter = frameLow.x.isFinite ? (frameLow + frameHigh) * 0.5 : target
        let sceneDistance = chunks.isEmpty ? distance : simd_length(eye - sceneCenter)
        let near = max(0.01, sceneDistance - radius * 1.1)
        let far = max(near + 1, sceneDistance + radius * 1.1)
        let scale = 1 / tan(Float.pi / 8)
        let aspect = Float(size.width / max(size.height, 1))
        let projection = simd_float4x4(columns: (
            SIMD4(scale / aspect, 0, 0, 0), SIMD4(0, scale, 0, 0),
            SIMD4(0, 0, far / (near - far), -1), SIMD4(0, 0, near * far / (near - far), 0)
        ))
        return projection * view
    }

    private func frustumPlanes(_ matrix: simd_float4x4) -> [SIMD4<Float>] {
        let rows = matrix.transpose
        return [rows[3] + rows[0], rows[3] - rows[0], rows[3] + rows[1],
                rows[3] - rows[1], rows[2], rows[3] - rows[2]]
    }
}

@MainActor
final class SchematicMetalView: MTKView {
    weak var renderer: SchematicMetalRenderer?
    override var acceptsFirstResponder: Bool { true }

    override func mouseDown(with event: NSEvent) {
        window?.makeFirstResponder(self)
        if event.clickCount == 2 { renderer?.resetCamera() }
    }
    override func mouseDragged(with event: NSEvent) {
        if event.modifierFlags.contains(.shift) {
            renderer?.pan(dx: Float(event.deltaX), dy: Float(event.deltaY))
        } else {
            renderer?.orbit(dx: Float(event.deltaX), dy: Float(event.deltaY))
        }
    }
    override func rightMouseDown(with event: NSEvent) { window?.makeFirstResponder(self) }
    override func rightMouseDragged(with event: NSEvent) {
        renderer?.pan(dx: Float(event.deltaX), dy: Float(event.deltaY))
    }
    override func otherMouseDragged(with event: NSEvent) {
        renderer?.pan(dx: Float(event.deltaX), dy: Float(event.deltaY))
    }
    override func scrollWheel(with event: NSEvent) {
        renderer?.zoom(Float(event.scrollingDeltaY) * (event.hasPreciseScrollingDeltas ? 0.005 : 0.08))
    }
    override func magnify(with event: NSEvent) { renderer?.zoom(-Float(event.magnification)) }
    override func keyDown(with event: NSEvent) {
        switch event.keyCode {
        case 123: renderer?.orbit(dx: -10, dy: 0)
        case 124: renderer?.orbit(dx: 10, dy: 0)
        case 125: renderer?.orbit(dx: 0, dy: 10)
        case 126: renderer?.orbit(dx: 0, dy: -10)
        default:
            switch event.charactersIgnoringModifiers?.lowercased() {
            case "r", "0": renderer?.resetCamera()
            case "+", "=": renderer?.zoom(-0.15)
            case "-": renderer?.zoom(0.15)
            default: super.keyDown(with: event)
            }
        }
    }
    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        effectiveAppearance.performAsCurrentDrawingAppearance {
            let color = NSColor.windowBackgroundColor.usingColorSpace(.deviceRGB)!
            clearColor = MTLClearColor(red: color.redComponent, green: color.greenComponent,
                                       blue: color.blueComponent, alpha: 1)
        }
        needsDisplay = true
    }
}
