import Foundation
import LitematicaQLNative

/// A content-level preview failure with panel-ready user-facing text.
struct NativeSchematicRefusal: Error, Sendable {
    let message: String
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

struct NativeMeshResult: Sendable {
    let triangleCount: Int
    let glb: Data
}

/// Owns a native decode-and-mesh handle.
///
/// All handle operations, including release, must remain within one synchronous task
/// segment. Never share the session across tasks or let it cross an `await`.
final class NativeSchematicSession {
    private var handle: OpaquePointer?

    deinit {
        if let handle {
            nql_schematic_free(handle)
        }
    }

    /// Opens `data` as the current schematic, throwing a panel-ready refusal for
    /// content that the native bridge cannot decode.
    func decode(_ data: Data) throws -> NativeSchematicFacts {
        var handle: OpaquePointer?
        var failure = NQLError(message: nil, message_len: 0)
        let status = data.withUnsafeBytes { raw -> NQLStatus in
            guard let base = raw.baseAddress else {
                return NQL_ERR_NULL
            }
            return nql_schematic_open(
                base.assumingMemoryBound(to: UInt8.self),
                raw.count,
                &handle,
                &failure
            )
        }
        guard status == NQL_OK else {
            throw Self.refusal(status, failure)
        }
        self.handle = handle

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
            contentDimensions: (
                Int(info.content_x),
                Int(info.content_y),
                Int(info.content_z)
            ),
            warnings: readWarnings()
        )
    }

    /// Returns notices for the current schematic. A failed or empty native read is
    /// represented as no warnings and does not invalidate the decoded schematic.
    func readWarnings() -> [String] {
        guard let handle else {
            return []
        }

        var buffer: UnsafeMutablePointer<UInt8>?
        var length = 0
        guard nql_schematic_warnings(handle, &buffer, &length) == NQL_OK else {
            return []
        }
        defer {
            if let buffer {
                nql_buffer_free(buffer, length)
            }
        }
        guard length > 0, let buffer else {
            return []
        }
        let text = String(
            decoding: UnsafeBufferPointer(start: buffer, count: length),
            as: UTF8.self
        )
        return text.split(separator: "\n", omittingEmptySubsequences: true).map(String.init)
    }

    /// Meshes the current schematic. The caller must have completed `decode(_:)` first.
    func mesh(pack: Data) throws -> NativeMeshResult {
        guard let handle else {
            throw NativeSchematicRefusal(
                message: "Something went wrong while reading this schematic."
            )
        }

        var glb: UnsafeMutablePointer<UInt8>?
        var glbLength = 0
        var info = NQLMeshInfo(triangle_count: 0)
        var failure = NQLError(message: nil, message_len: 0)
        let status = pack.withUnsafeBytes { raw -> NQLStatus in
            guard let base = raw.baseAddress else {
                return NQL_ERR_NULL
            }
            return nql_schematic_mesh(
                handle,
                base.assumingMemoryBound(to: UInt8.self),
                raw.count,
                &glb,
                &glbLength,
                &info,
                &failure
            )
        }
        guard status == NQL_OK else {
            throw Self.refusal(status, failure)
        }

        defer {
            if let glb {
                nql_buffer_free(glb, glbLength)
            }
        }
        guard let glb, glbLength > 0 else {
            throw NativeSchematicRefusal(
                message: "Something went wrong while reading this schematic."
            )
        }
        let meshData = Data(bytes: glb, count: glbLength)
        return NativeMeshResult(triangleCount: Int(info.triangle_count), glb: meshData)
    }

    private static func refusal(
        _ status: NQLStatus,
        _ failure: NQLError
    ) -> NativeSchematicRefusal {
        defer {
            if let message = failure.message {
                nql_buffer_free(message, failure.message_len)
            }
        }

        if let bytes = failure.message, failure.message_len > 0 {
            let message = String(decoding: Data(bytes: bytes, count: failure.message_len), as: UTF8.self)
            if !message.isEmpty {
                return NativeSchematicRefusal(message: message)
            }
        }

        switch status {
        case NQL_ERR_FORMAT:
            return NativeSchematicRefusal(
                message: "This file is not a readable Minecraft schematic, or it expands beyond the 1024 MiB preview limit."
            )
        case NQL_ERR_LIMIT:
            return NativeSchematicRefusal(
                message: "This schematic is too large to preview."
            )
        case NQL_ERR_NO_BLOCKS:
            return NativeSchematicRefusal(
                message: "This schematic contains no blocks to render."
            )
        case NQL_ERR_MESH:
            return NativeSchematicRefusal(
                message: "This schematic has too much visible surface to preview. Its block geometry exceeds what the renderer can build."
            )
        case NQL_ERR_PACK:
            return NativeSchematicRefusal(
                message: "The bundled block resources are invalid. Rebuild the renderer assets and the app."
            )
        default:
            return NativeSchematicRefusal(
                message: "Something went wrong while reading this schematic."
            )
        }
    }
}
