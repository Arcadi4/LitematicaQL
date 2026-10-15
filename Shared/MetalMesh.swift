import Foundation
import LitematicaQLNative
import Metal
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
    private var textures: [UInt32: MTLTexture] = [:]
    private let uploads = DispatchSemaphore(value: 2)
    private let errorLock = NSLock()
    private var uploadError: String?

    init(device: MTLDevice, queue: MTLCommandQueue) {
        self.device = device
        self.queue = queue
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
        let staging = try start.atlas.withUnsafeBytes { raw -> MTLBuffer in
            guard let base = raw.baseAddress else { throw allocationError() }
            return try self.staging(base, rowBytes: rowBytes, width: start.atlasWidth,
                                    height: start.atlasHeight)
        }
        guard let command = queue.makeCommandBuffer(), let blit = command.makeBlitCommandEncoder() else {
            throw allocationError()
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
              let command = queue.makeCommandBuffer(),
              let blit = command.makeBlitCommandEncoder() else { throw allocationError() }
        buffer.label = "Schematic chunk \(batch.info.batch_index)"
        // First-use RGBA textures upload ahead of the geometry copy, so every
        // draw committed after this batch finds the pixels it samples.
        for texture in UnsafeBufferPointer(start: view.textures, count: view.texture_count) {
            try upload(texture, with: blit)
        }
        var parts: [MetalMeshPart] = []
        parts.reserveCapacity(view.part_count)
        for part in UnsafeBufferPointer(start: view.parts, count: view.part_count) {
            guard let texture = textures[part.texture_index] else {
                throw NativeSchematicRefusal(message: "A mesh chunk is missing its block texture.")
            }
            parts.append(MetalMeshPart(
                vertexOffset: part.vertex_offset, indexOffset: part.index_offset,
                indexCount: Int(part.index_count), indexType: part.index_size == 2 ? .uint16 : .uint32,
                alphaMode: Int(part.alpha_mode), texture: texture, repeats: part.texture_index != 0
            ))
        }
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

    /// Copies unpadded RGBA rows into a shared staging buffer, padding each row
    /// to the 256-byte alignment blit copies expect. The caller keeps the
    /// source pixels alive until this returns; the command buffer keeps the
    /// staging buffer alive until the copy completes.
    private func staging(
        _ pixels: UnsafeRawPointer, rowBytes: Int, width: Int, height: Int
    ) throws -> MTLBuffer {
        guard let staging = device.makeBuffer(length: rowBytes * height, options: .storageModeShared)
        else { throw allocationError() }
        if rowBytes == width * 4 {
            staging.contents().copyMemory(from: pixels, byteCount: rowBytes * height)
        } else {
            for y in 0..<height {
                staging.contents().advanced(by: y * rowBytes).copyMemory(
                    from: pixels.advanced(by: y * width * 4), byteCount: width * 4
                )
            }
        }
        return staging
    }

    /// Uploads one batch-owned RGBA8 texture as private shader storage. The
    /// sampler's mip filter needs the chain repeating textures carry, so they
    /// upload mipmapped and blit generates the levels; the clamped atlas
    /// keeps the mip-free upload it got at stream open.
    private func upload(_ texture: NQLBatchTexture, with blit: MTLBlitCommandEncoder) throws {
        let width = Int(texture.width)
        let height = Int(texture.height)
        let descriptor = MTLTextureDescriptor.texture2DDescriptor(
            pixelFormat: .rgba8Unorm_srgb, width: width, height: height,
            mipmapped: texture.repeats
        )
        descriptor.storageMode = .private
        descriptor.usage = .shaderRead
        guard let target = device.makeTexture(descriptor: descriptor) else { throw allocationError() }
        target.label = "Schematic texture \(texture.index)"
        let rowBytes = (width * 4 + 255) & ~255
        guard let source = texture.pixels else { throw allocationError() }
        let staging = try staging(source, rowBytes: rowBytes, width: width, height: height)
        blit.copy(from: staging, sourceOffset: 0, sourceBytesPerRow: rowBytes,
                  sourceBytesPerImage: rowBytes * height,
                  sourceSize: MTLSize(width: width, height: height, depth: 1),
                  to: target, destinationSlice: 0, destinationLevel: 0,
                  destinationOrigin: MTLOrigin(x: 0, y: 0, z: 0))
        if texture.repeats { blit.generateMipmaps(for: target) }
        textures[texture.index] = target
    }

    func finish() throws {
        // Queue ordering also covers the atlas and first-use texture uploads.
        // This checks completion without depending on MTKView visibility.
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
