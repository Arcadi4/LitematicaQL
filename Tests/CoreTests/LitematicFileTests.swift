import Foundation
import Testing
@testable import LitematicaQLCore

@Suite("Litematica file validation")
struct LitematicFileTests {
    @Test("Supports every registered extension", arguments: LitematicFile.supportedFileExtensions)
    func acceptsSupportedExtension(fileExtension: String) throws {
        let url = try temporaryFile(named: "schematic.\(fileExtension)", bytes: [0x1f, 0x8b])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let data = try LitematicFile.readValidatedData(from: url)

        #expect(data == Data([0x1f, 0x8b]))
    }

    @Test("Accepts an upper-case extension")
    func acceptsUpperCaseExtension() throws {
        let url = try temporaryFile(named: "schematic.LITEMATIC", bytes: [0x1f, 0x8b])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let data = try LitematicFile.readValidatedData(from: url)

        #expect(data == Data([0x1f, 0x8b]))
    }

    // Extension and size checks are the only validation here; the bridge owns
    // format interpretation, so otherwise valid bytes must remain untouched.
    @Test("Accepts arbitrary content")
    func acceptsArbitraryContent() throws {
        let url = try temporaryFile(named: "schematic.nbt", bytes: [0x0a, 0x00, 0xff, 0x00])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let data = try LitematicFile.readValidatedData(from: url)

        #expect(data == Data([0x0a, 0x00, 0xff, 0x00]))
    }

    @Test("Rejects an unsupported extension")
    func rejectsOtherExtension() throws {
        let url = try temporaryFile(named: "invalid.txt", bytes: [0x1f, 0x8b])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.unsupportedExtension) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    @Test("Rejects a missing extension")
    func rejectsMissingExtension() throws {
        let url = try temporaryFile(named: "schematic", bytes: [0x1f, 0x8b])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.unsupportedExtension) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    @Test("Names every supported extension in the rejection message")
    func namesSupportedExtensions() {
        let message = LitematicFileError.unsupportedExtension.errorDescription ?? ""

        for fileExtension in LitematicFile.supportedFileExtensions {
            #expect(message.contains(".\(fileExtension)"))
        }
    }

    @Test("Rejects an empty file")
    func rejectsEmptyFile() throws {
        let url = try temporaryFile(named: "empty.litematic", bytes: [])
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.emptyFile) {
            try LitematicFile.readValidatedData(from: url)
        }
    }

    @Test("Accepts a file at the preview limit")
    func acceptsFileAtSizeLimit() throws {
        let url = try temporarySparseFile(
            named: "limit.litematic",
            size: LitematicFile.maximumFileSize
        )
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        let data = try LitematicFile.readValidatedData(from: url)

        #expect(data.count == LitematicFile.maximumFileSize)
    }

    @Test("Rejects a file above the preview limit")
    func rejectsFileAboveSizeLimit() throws {
        let url = try temporarySparseFile(
            named: "too-large.litematic",
            size: LitematicFile.maximumFileSize + 1
        )
        defer { try? FileManager.default.removeItem(at: url.deletingLastPathComponent()) }

        #expect(throws: LitematicFileError.fileTooLarge(maximumBytes: LitematicFile.maximumFileSize)) {
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
