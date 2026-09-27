import Foundation
import MetalKit
import simd

struct MetalMeshPart: @unchecked Sendable {
    let vertexOffset: Int
    let indexOffset: Int
    let indexCount: Int
    let indexType: MTLIndexType
    let alphaMode: Int
    let texture: MTLTexture
    let repeats: Bool
}

struct MetalMeshChunk: @unchecked Sendable {
    let buffer: MTLBuffer
    let parts: [MetalMeshPart]
    let origin: SIMD3<Float>
    let extent: SIMD3<Float>
    var center: SIMD3<Float> { origin + extent * 0.5 }
}

/// Used exclusively by a background producer. Two uploads may be in flight;
/// the next chunk can mesh while the GPU copies the preceding chunk. Geometry
/// becomes private GPU storage, and command completion releases staging buffers.
final class MetalMeshUploader: @unchecked Sendable {
    let device: MTLDevice
    let queue: MTLCommandQueue
    private let loader: MTKTextureLoader
    private var textures: [UInt32: MTLTexture] = [:]
    private let uploads = DispatchSemaphore(value: 2)
    private let errorLock = NSLock()
    private var uploadError: String?

    init(device: MTLDevice, queue: MTLCommandQueue) {
        self.device = device
        self.queue = queue
        loader = MTKTextureLoader(device: device)
    }

    func uploadAtlas(_ start: NativeMeshStart) throws {
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .rgba8Unorm_srgb, width: start.atlasWidth,
            height: start.atlasHeight, mipmapped: false
        )
        descriptor.storageMode = .private
        descriptor.usage = .shaderRead
        descriptor.allowGPUOptimizedContents = true
        guard let texture = device.makeTexture(descriptor: descriptor) else { throw allocationError() }
        texture.label = "Schematic atlas"
        // One staging copy from the Rust-owned pixels. Avoid image decoding and
        // leave private, shader-read-only textures eligible for GPU compression.
        let rowBytes = (start.atlasWidth * 4 + 255) & ~255
        guard let staging = device.makeBuffer(length: rowBytes * start.atlasHeight, options: .storageModeShared),
              let command = queue.makeCommandBuffer(), let blit = command.makeBlitCommandEncoder() else {
            throw allocationError()
        }
        start.atlas.withUnsafeBytes { source in
            for y in 0..<start.atlasHeight {
                staging.contents().advanced(by: y * rowBytes).copyMemory(
                    from: source.baseAddress!.advanced(by: y * start.atlasWidth * 4),
                    byteCount: start.atlasWidth * 4
                )
            }
        }
        blit.copy(from: staging, sourceOffset: 0, sourceBytesPerRow: rowBytes,
                  sourceBytesPerImage: rowBytes * start.atlasHeight,
                  sourceSize: MTLSize(width: start.atlasWidth, height: start.atlasHeight, depth: 1),
                  to: texture, destinationSlice: 0, destinationLevel: 0,
                  destinationOrigin: MTLOrigin(x: 0, y: 0, z: 0))
        blit.endEncoding()
        command.commit()
        command.waitUntilCompleted()
        if let error = command.error { throw error }
        textures[0] = texture
    }

    func upload(_ batch: NativeMeshBatch) throws -> MetalMeshChunk? {
        try Task.checkCancellation()
        let view = batch.view
        if view.geometry_length == 0 { return nil }
        uploads.wait()
        // Release the slot on failure; successful submissions transfer it to
        // the GPU completion handler. No task waits for an on-screen draw.
        var submitted = false
        defer { if !submitted { uploads.signal() } }
        try Task.checkCancellation()
        try checkError()
        guard let data = view.geometry,
              let staging = device.makeBuffer(bytes: data, length: view.geometry_length, options: .storageModeShared),
              let buffer = device.makeBuffer(length: view.geometry_length, options: .storageModePrivate),
              let command = queue.makeCommandBuffer() else { throw allocationError() }
        buffer.label = "Schematic chunk \(batch.info.batch_index)"
        var parts: [MetalMeshPart] = []
        for part in UnsafeBufferPointer(start: view.parts, count: view.part_count) {
            if let png = part.texture_png, part.texture_length > 0 {
                // The mesher exposes greedy textures as PNG. Borrow their bytes
                // for the synchronous decoder; never retain native batch memory.
                let data = Data(bytesNoCopy: UnsafeMutableRawPointer(mutating: png),
                                count: part.texture_length, deallocator: .none)
                textures[part.texture_index] = try loader.newTexture(data: data, options: [
                    .SRGB: true, .generateMipmaps: true,
                    .textureUsage: NSNumber(value: MTLTextureUsage.shaderRead.rawValue),
                    .textureStorageMode: NSNumber(value: MTLStorageMode.private.rawValue),
                    .origin: MTKTextureLoader.Origin.topLeft,
                ])
            }
            guard let texture = textures[part.texture_index] else {
                throw NativeSchematicRefusal(message: "A mesh chunk is missing its block texture.")
            }
            parts.append(MetalMeshPart(
                vertexOffset: part.vertex_offset, indexOffset: part.index_offset,
                indexCount: Int(part.index_count), indexType: part.index_size == 2 ? .uint16 : .uint32,
                alphaMode: Int(part.alpha_mode), texture: texture, repeats: part.texture_index != 0
            ))
        }
        guard let blit = command.makeBlitCommandEncoder() else { throw allocationError() }
        blit.copy(from: staging, sourceOffset: 0, to: buffer, destinationOffset: 0, size: view.geometry_length)
        blit.endEncoding()
        command.addCompletedHandler { [self] command in
            if let error = command.error {
                errorLock.lock()
                uploadError = error.localizedDescription
                errorLock.unlock()
            }
            uploads.signal()
        }
        command.commit()
        submitted = true
        return MetalMeshChunk(buffer: buffer, parts: parts,
                              origin: SIMD3(view.origin.0, view.origin.1, view.origin.2),
                              extent: SIMD3(view.extent.0, view.extent.1, view.extent.2))
    }

    func finish() throws {
        // Queue ordering also covers atlas and texture-loader uploads. This
        // checks completion without depending on MTKView visibility.
        uploads.wait()
        uploads.wait()
        uploads.signal()
        uploads.signal()
        try checkError()
    }

    private func checkError() throws {
        errorLock.lock()
        let error = uploadError
        errorLock.unlock()
        if let error { throw NativeSchematicRefusal(message: error) }
    }

    private func allocationError() -> NativeSchematicRefusal {
        NativeSchematicRefusal(message: "There isn’t enough graphics memory to preview this schematic.")
    }
}
