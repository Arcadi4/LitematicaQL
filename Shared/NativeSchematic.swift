import Foundation
import LitematicaQLNative

/// A content-level preview failure with panel-ready user-facing text.
struct NativeSchematicRefusal: LocalizedError, Sendable {
    let message: String
    var errorDescription: String? { message }
}

/// Decoded build metadata exposed before meshing begins.
struct NativeSchematicFacts: Sendable {
    let blockCount: Int
    let blockEntityCount: Int
    /// Occupied-block bounds; region padding can make declared dimensions larger.
    let contentDimensions: (Int, Int, Int)
    /// Notices for content that exists but cannot be displayed, such as unreadable
    /// chunks or regions held in external `.mcc` files.
    let warnings: [String]
}

struct NativeMeshStart: Sendable {
    let batchCount: Int
    let atlas: Data
    let atlasWidth: Int
    let atlasHeight: Int
}

/// Owns one native allocation until Metal has copied the compact geometry.
final class NativeMeshBatch: @unchecked Sendable {
    let handle: OpaquePointer
    let info: NQLBatchInfo

    init(handle: OpaquePointer, info: NQLBatchInfo) {
        self.handle = handle
        self.info = info
    }

    var view: NQLBatchView { nql_mesh_batch_view(handle) }
    deinit { nql_mesh_batch_free(handle) }
}

/// Owns a parsed immutable resource pack. One instance may be reused by any
/// number of previews; the native pack is immutable after this call returns.
final class NativeResourcePack: @unchecked Sendable {
    private let handle: OpaquePointer

    // Parsing is expensive and the resource pack is immutable. Swift's static
    // initialization serializes the first access across preview workers.
    static let bundled: Result<NativeResourcePack, Error> = Result {
        guard let url = Bundle.main.url(forResource: "pack", withExtension: "zip") else {
            throw NativeSchematicRefusal(message: "The bundled block resources are missing.")
        }
        return try NativeResourcePack(Data(contentsOf: url, options: .mappedIfSafe))
    }

    init(_ data: Data) throws {
        var handle: OpaquePointer?
        var failure = NQLError(message: nil, message_len: 0)
        let status = data.withUnsafeBytes { raw -> NQLStatus in
            guard let base = raw.baseAddress else { return NQL_ERR_NULL }
            return nql_resource_pack_open(
                base.assumingMemoryBound(to: UInt8.self),
                raw.count,
                &handle,
                &failure
            )
        }
        guard status == NQL_OK, let handle else {
            throw NativeSchematicRefusal(message: Self.packMessage(status, failure))
        }
        self.handle = handle
    }

    deinit {
        nql_resource_pack_free(handle)
    }

    fileprivate func withHandle<T>(_ body: (OpaquePointer) throws -> T) rethrows -> T {
        try body(handle)
    }

    private static func packMessage(_ status: NQLStatus, _ failure: NQLError) -> String {
        defer { if let message = failure.message { nql_buffer_free(message, failure.message_len) } }
        if let message = failure.message, failure.message_len > 0 {
            let text = String(decoding: Data(bytes: message, count: failure.message_len), as: UTF8.self)
            if !text.isEmpty { return text }
        }
        return status == NQL_ERR_PACK
            ? "The bundled block resources are invalid. Rebuild the app."
            : "Something went wrong while reading the bundled block resources."
    }
}

/// Owns one native decoded-preview handle.
///
/// The handle may be cancelled from Swift structured concurrency while native
/// meshing is active. Decoding, querying, meshing, and release remain inside
/// synchronous calls; only cancellation may overlap a mesh call.
final class NativeSchematicSession: @unchecked Sendable {
    private var handle: OpaquePointer?
    private var meshStream: OpaquePointer?
    private var resourcePack: NativeResourcePack?
    private let cancellationLock = NSLock()
    private var cancelled = false

    deinit {
        endMesh()
        if let handle { nql_schematic_free(handle) }
    }

    func decode(_ data: Data) throws -> NativeSchematicFacts {
        var handle: OpaquePointer?
        var failure = NQLError(message: nil, message_len: 0)
        let status = data.withUnsafeBytes { raw -> NQLStatus in
            guard let base = raw.baseAddress else { return NQL_ERR_NULL }
            return nql_schematic_open(
                base.assumingMemoryBound(to: UInt8.self),
                raw.count,
                &handle,
                &failure
            )
        }
        guard status == NQL_OK, let handle else {
            throw Self.refusal(status, failure)
        }
        cancellationLock.lock()
        self.handle = handle
        let wasCancelled = cancelled
        if wasCancelled { nql_schematic_cancel(handle) }
        cancellationLock.unlock()
        if wasCancelled { throw CancellationError() }

        var info = NQLSchematicInfo(
            block_count: 0,
            block_entity_count: 0,
            content_x: 0,
            content_y: 0,
            content_z: 0
        )
        guard nql_schematic_info(handle, &info) == NQL_OK else {
            throw NativeSchematicRefusal(
                message: "Something went wrong while reading this schematic."
            )
        }
        return NativeSchematicFacts(
            blockCount: Int(info.block_count),
            blockEntityCount: Int(info.block_entity_count),
            contentDimensions: (Int(info.content_x), Int(info.content_y), Int(info.content_z)),
            warnings: readWarnings()
        )
    }

    func cancel() {
        cancellationLock.lock()
        cancelled = true
        if let handle { nql_schematic_cancel(handle) }
        cancellationLock.unlock()
    }

    func readWarnings() -> [String] {
        guard let handle else { return [] }
        var buffer: UnsafeMutablePointer<UInt8>?
        var length = 0
        guard nql_schematic_warnings(handle, &buffer, &length) == NQL_OK else { return [] }
        defer { if let buffer { nql_buffer_free(buffer, length) } }
        guard length > 0, let buffer else { return [] }
        let text = String(
            decoding: UnsafeBufferPointer(start: buffer, count: length),
            as: UTF8.self
        )
        return text.split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
    }

    func beginMesh(pack: NativeResourcePack) throws -> NativeMeshStart {
        guard let handle else {
            throw NativeSchematicRefusal(message: "Something went wrong while reading this schematic.")
        }
        guard meshStream == nil else {
            throw NativeSchematicRefusal(message: "A mesh stream is already active.")
        }
        return try pack.withHandle { packHandle in
            var atlas: UnsafeMutablePointer<UInt8>?
            var atlasLength = 0
            var atlasInfo = NQLAtlasInfo(width: 0, height: 0)
            var meshInfo = NQLMeshInfo(batch_count: 0, triangle_count: 0)
            var stream: OpaquePointer?
            var failure = NQLError(message: nil, message_len: 0)
            let status = nql_mesh_stream_open(
                handle,
                packHandle,
                &atlas,
                &atlasLength,
                &atlasInfo,
                &meshInfo,
                &stream,
                &failure
            )
            if status == NQL_ERR_CANCELLED {
                if let message = failure.message { nql_buffer_free(message, failure.message_len) }
                throw CancellationError()
            }
            guard status == NQL_OK, let atlas, let stream, atlasLength > 0 else {
                throw Self.refusal(status, failure)
            }
            let data = Data(bytesNoCopy: atlas, count: atlasLength, deallocator: .custom { pointer, _ in
                nql_buffer_free(pointer.assumingMemoryBound(to: UInt8.self), atlasLength)
            })
            meshStream = stream
            resourcePack = pack
            return NativeMeshStart(
                batchCount: Int(meshInfo.batch_count),
                atlas: data,
                atlasWidth: Int(atlasInfo.width),
                atlasHeight: Int(atlasInfo.height)
            )
        }
    }

    func nextBatch(index: Int) throws -> NativeMeshBatch? {
        guard let meshStream else {
            throw NativeSchematicRefusal(message: "The native mesh stream is not available.")
        }
        var batch: OpaquePointer?
        var info = NQLBatchInfo()
        var failure = NQLError(message: nil, message_len: 0)
        let status = nql_mesh_stream_next(meshStream, UInt32(index), &batch, &info, &failure)
        if status == NQL_DONE { return nil }
        if status == NQL_ERR_CANCELLED {
            if let message = failure.message { nql_buffer_free(message, failure.message_len) }
            throw CancellationError()
        }
        guard status == NQL_OK, let batch else { throw Self.refusal(status, failure) }
        return NativeMeshBatch(handle: batch, info: info)
    }

    // Only the producer uses or releases the stream; cancellation touches just
    // the schematic's atomic flag and never frees memory under a mesh call.
    func endMesh() {
        if let meshStream { nql_mesh_stream_free(meshStream) }
        meshStream = nil
        resourcePack = nil
    }

    private static func refusal(_ status: NQLStatus, _ failure: NQLError) -> NativeSchematicRefusal {
        defer { if let message = failure.message { nql_buffer_free(message, failure.message_len) } }
        if let bytes = failure.message, failure.message_len > 0 {
            let message = String(decoding: Data(bytes: bytes, count: failure.message_len), as: UTF8.self)
            if !message.isEmpty { return NativeSchematicRefusal(message: message) }
        }
        switch status {
        case NQL_ERR_FORMAT:
            return NativeSchematicRefusal(
                message: "This file is not a readable Minecraft schematic, or it expands beyond the 1024 MiB preview limit."
            )
        case NQL_ERR_LIMIT:
            return NativeSchematicRefusal(message: "This schematic is too large to preview.")
        case NQL_ERR_NO_BLOCKS:
            return NativeSchematicRefusal(message: "This schematic contains no blocks to render.")
        case NQL_ERR_MESH:
            return NativeSchematicRefusal(
                message: "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build."
            )
        case NQL_ERR_PACK:
            return NativeSchematicRefusal(
                message: "The bundled block resources are invalid. Rebuild the app."
            )
        default:
            return NativeSchematicRefusal(
                message: "Something went wrong while reading this schematic."
            )
        }
    }
}
