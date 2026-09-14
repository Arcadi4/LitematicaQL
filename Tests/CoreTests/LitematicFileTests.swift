// LitematicaQL: macOS Quick Look plugin for Litematica schematics.
// Copyright (C) 2026 4rcadia
// SPDX-License-Identifier: AGPL-3.0-or-later
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program. If not, see <https://www.gnu.org/licenses/>.
//
// See the LICENSE file for the full license text.

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

    /**
     Content is the renderer's business: several supported file formats are
     uncompressed and every format decision belongs behind the bridge, so the
     gate must accept bytes it cannot interpret.
     */
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
