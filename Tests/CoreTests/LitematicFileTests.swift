import Foundation
import Testing
@testable import LitematicaQLCore

@Suite("Litematica file validation")
struct LitematicFileTests {
    @Test("Accepts gzip-compressed Litematica data")
    func acceptsGzipData() throws {
        let url = try temporaryFile(named: "valid.litematic", bytes: [0x1f, 0x8b, 0x08, 0x00])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let data = try LitematicFile.readValidatedData(from: url)

        #expect(data == Data([0x1f, 0x8b, 0x08, 0x00]))
    }

    @Test("Rejects a different extension")
    func rejectsOtherExtension() throws {
        let url = try temporaryFile(named: "invalid.schem", bytes: [0x1f, 0x8b])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.self) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    @Test("Rejects uncompressed data")
    func rejectsInvalidHeader() throws {
        let url = try temporaryFile(named: "invalid.litematic", bytes: [0x0a, 0x00])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.self) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    @Test("Rejects an empty file")
    func rejectsEmptyFile() throws {
        let url = try temporaryFile(named: "empty.litematic", bytes: [])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.self) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    @Test("Accepts a file at the compressed preview limit")
    func acceptsFileAtSizeLimit() throws {
        let url = try temporarySparseFile(
            named: "limit.litematic",
            size: LitematicFile.maximumCompressedFileSize
        )
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let data = try LitematicFile.readValidatedData(from: url)

        #expect(data.count == LitematicFile.maximumCompressedFileSize)
    }

    @Test("Rejects a file above the compressed preview limit")
    func rejectsFileAboveSizeLimit() throws {
        let url = try temporarySparseFile(
            named: "too-large.litematic",
            size: LitematicFile.maximumCompressedFileSize + 1
        )
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(
            throws: LitematicFileError.fileTooLarge(
                maximumBytes: LitematicFile.maximumCompressedFileSize
            )
        ) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    private func temporaryFile(named name: String, bytes: [UInt8]) throws -> URL {
        let directory = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let url = directory.appendingPathComponent(name)
        try Data(bytes).write(to: url)
        return url
    }

    private func temporarySparseFile(named name: String, size: Int) throws -> URL {
        let url = try temporaryFile(named: name, bytes: [0x1f, 0x8b])
        let handle = try FileHandle(forWritingTo: url)
        defer { try? handle.close() }
        try handle.truncate(atOffset: UInt64(size))
        return url
    }
}
