// LitematicaQL: Quick Look preview for Minecraft schematics on macOS.
// Copyright (c) 2026 4rcadia
// SPDX-License-Identifier: MIT

import Foundation
import LitematicaQLNative

// Why a schematic cannot be shown, carrying the copy the preview panel
// reports.
struct NativeSchematicRefusal: Error, Sendable {
    let message: String
}

// Facts about a decoded build, reported to the preview UI before meshing.
struct NativeSchematicFacts: Sendable {
    let blockCount: Int
    let blockEntityCount: Int
    // Dimensions of the region the blocks actually occupy. Region padding is
    // a whole-chunk affair, so the declared size overstates what is drawn.
    let contentDimensions: (Int, Int, Int)
}

// Facts about a finished mesh.
struct NativeMeshResult: Sendable {
    let triangleCount: Int
    let glb: Data
}

// One decode-and-mesh session against the native bridge.
//
// The library handle is owned by the instance. The Quick Look flow keeps the
// handle on one task, since decode, mesh, and teardown all run inside a
// single background task. The pointer never crosses threads.
final class NativeSchematicSession {
    private var handle: OpaquePointer?

    deinit {
        if let handle {
            nql_schematic_free(handle)
        }
    }

    // Decodes schematic bytes, or throws a refusal with panel copy.
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
            )
        )
    }

    // Meshes the decoded schematic against the bundled resource pack.
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

    // Converts a native status plus error payload into a panel refusal.
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
