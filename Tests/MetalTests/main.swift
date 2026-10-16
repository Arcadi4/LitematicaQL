import Darwin
import Foundation
import LitematicaQLNative
import Metal

struct MetalTestFailure: Error, CustomStringConvertible {
    let description: String
}

func require(_ condition: @autoclosure () -> Bool, _ message: String) throws {
    guard condition() else { throw MetalTestFailure(description: message) }
}

final class BatchLifetimeProbe {
    weak var batch: NativeMeshBatch?
}

let fixturePaths = [
    "Fixtures/Demos/Archway.schem",
    "Fixtures/Demos/Bloom.nbt",
    "Fixtures/Demos/Bridge.snbt",
    "Fixtures/Demos/Cottage.litematic",
    "Fixtures/Demos/Garden.nusn",
    "Fixtures/Demos/Pyramid.mcstructure",
    "Fixtures/Demos/Watchtower.schematic",
    "Fixtures/Formats/Bedrock.mcstructure",
    "Fixtures/Formats/Classic.schematic",
    "Fixtures/Formats/Region.mca",
    "Fixtures/Formats/Snapshot.nusn",
    "Fixtures/Formats/Sponge.schem",
    "Fixtures/Formats/Structure.nbt",
    "Fixtures/Formats/Structure.snbt",
]

let repository = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let pageSize = Int(getpagesize())

func readback(_ chunk: MetalMeshChunk, expected: Data, device: MTLDevice,
              queue: MTLCommandQueue, context: String) throws {
    guard let destination = device.makeBuffer(length: expected.count, options: .storageModeShared),
          let command = queue.makeCommandBuffer(),
          let blit = command.makeBlitCommandEncoder() else {
        throw MetalTestFailure(description: "\(context): could not allocate GPU readback resources")
    }
    blit.copy(from: chunk.buffer, sourceOffset: 0, to: destination, destinationOffset: 0,
              size: expected.count)
    blit.endEncoding()
    command.commit()
    command.waitUntilCompleted()
    if let error = command.error {
        throw MetalTestFailure(description: "\(context): GPU readback failed: \(error)")
    }
    let actual = Data(bytes: destination.contents(), count: expected.count)
    try require(actual == expected, "\(context): GPU bytes differ from native geometry")
}

func runFixture(_ path: String, pack: NativeResourcePack, device: MTLDevice,
                queue: MTLCommandQueue, smallPayloadCount: inout Int,
                batchCount: inout Int) throws {
    let url = repository.appendingPathComponent(path)
    let file = try Data(contentsOf: url, options: .mappedIfSafe)
    let session = NativeSchematicSession(overridesPreviewLimits: true)
    _ = try session.decode(file)
    let start = try session.beginMesh(pack: pack)
    let uploader = MetalMeshUploader(device: device, queue: queue)
    try uploader.uploadAtlas(start)

    for index in 0..<start.batchCount {
        guard let batch = try session.nextBatch(index: index) else {
            throw MetalTestFailure(description: "\(path): missing mesh batch \(index)")
        }
        let view = batch.view
        guard let geometry = view.geometry, view.geometry_length > 0 else {
            throw MetalTestFailure(description: "\(path): batch \(index) has no geometry")
        }
        let expected = Data(bytes: UnsafeRawPointer(geometry), count: view.geometry_length)
        if expected.count < pageSize { smallPayloadCount += 1 }
        guard let chunk = try uploader.upload(batch) else {
            throw MetalTestFailure(description: "\(path): uploader dropped batch \(index)")
        }
        try readback(chunk, expected: expected, device: device, queue: queue,
                     context: "\(path) batch \(index)")
        batchCount += 1
    }

    session.endMesh()
    try uploader.finish()
}

func hasBorrowedInterior(_ batch: NativeMeshBatch) -> Bool {
    let view = batch.view
    guard let geometry = view.geometry, view.geometry_length > 0 else { return false }
    let address = UInt(bitPattern: UnsafeRawPointer(geometry))
    let head = (pageSize - Int(address % UInt(pageSize))) % pageSize
    return view.geometry_length > head
        && (view.geometry_length - head) / pageSize > 0
}

func proveBorrowLifetime(pack: NativeResourcePack, device: MTLDevice,
                         queue: MTLCommandQueue) throws {
    for path in fixturePaths {
        let file = try Data(contentsOf: repository.appendingPathComponent(path), options: .mappedIfSafe)
        var session: NativeSchematicSession? = NativeSchematicSession(overridesPreviewLimits: true)
        _ = try session!.decode(file)
        let start = try session!.beginMesh(pack: pack)
        let uploader = MetalMeshUploader(device: device, queue: queue)
        try uploader.uploadAtlas(start)

        for index in 0..<start.batchCount {
            var batch: NativeMeshBatch? = try session!.nextBatch(index: index)
            guard batch != nil else {
                throw MetalTestFailure(description: "\(path): missing mesh batch \(index)")
            }
            guard hasBorrowedInterior(batch!) else {
                batch = nil
                continue
            }
            let view = batch!.view
            guard let geometry = view.geometry else {
                throw MetalTestFailure(description: "\(path): batch \(index) has no geometry")
            }
            let expected = Data(bytes: UnsafeRawPointer(geometry), count: view.geometry_length)
            guard let event = device.makeSharedEvent(),
                  let gate = queue.makeCommandBuffer() else {
                throw MetalTestFailure(description: "Metal shared-event queue gating is unavailable")
            }
            event.label = "Metal staging lifetime gate"
            gate.encodeWaitForEvent(event, value: 1)
            gate.commit()
            defer { event.signaledValue = 1 }

            let probe = BatchLifetimeProbe()
            probe.batch = batch
            guard let chunk = try autoreleasepool(invoking: { try uploader.upload(batch!) }) else {
                throw MetalTestFailure(description: "\(path): uploader dropped batch \(index)")
            }
            batch = nil
            session!.endMesh()
            session = nil
            try require(probe.batch != nil,
                        "\(path) batch \(index): native batch released before gated GPU completion")

            event.signaledValue = 1
            try uploader.finish()
            try readback(chunk, expected: expected, device: device, queue: queue,
                         context: "\(path) batch \(index) lifetime readback")
            try require(probe.batch == nil,
                        "\(path) batch \(index): native batch remained alive after GPU completion")
            return
        }

        session!.endMesh()
        session = nil
        try uploader.finish()
    }
    throw MetalTestFailure(description: "No fixture batch contains a page-aligned borrowable interior")
}

func run() throws {
    guard let device = MTLCreateSystemDefaultDevice() else {
        throw MetalTestFailure(description: "No Metal device is available; run this opt-in suite on a Metal-capable Mac")
    }
    guard let queue = device.makeCommandQueue() else {
        throw MetalTestFailure(description: "Could not create a Metal command queue")
    }

    let packData = try Data(contentsOf: repository.appendingPathComponent("Resources/pack.zip"),
                            options: .mappedIfSafe)
    let pack = try NativeResourcePack(packData)
    var batchCount = 0
    var smallPayloadCount = 0
    for path in fixturePaths {
        try runFixture(path, pack: pack, device: device, queue: queue,
                       smallPayloadCount: &smallPayloadCount, batchCount: &batchCount)
    }
    try require(batchCount > 0, "Fixtures produced no mesh batches")
    try require(smallPayloadCount > 0, "Fixtures did not exercise sub-page geometry payloads")
    try proveBorrowLifetime(pack: pack, device: device, queue: queue)
    print("Metal staging passed: \(fixturePaths.count) fixtures, \(batchCount) batches, \(smallPayloadCount) sub-page payloads")
}

do {
    try run()
} catch {
    fputs("Metal staging failed: \(error)\n", stderr)
    exit(EXIT_FAILURE)
}
