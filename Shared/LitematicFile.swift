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

enum LitematicFileError: Equatable, LocalizedError {
    case emptyFile
    case fileTooLarge(maximumBytes: Int)
    case invalidHeader
    case notAFile
    case unreadable(String)
    case unsupportedExtension

    var errorDescription: String? {
        switch self {
        case .emptyFile:
            return "The schematic file is empty."
        case let .fileTooLarge(maximumBytes):
            let limit = ByteCountFormatter.string(fromByteCount: Int64(maximumBytes), countStyle: .binary)
            return "The schematic is too large to preview. LitematicaQL supports files up to \(limit)."
        case .invalidHeader:
            return "The file is not a valid gzip-compressed Litematica schematic."
        case .notAFile:
            return "The selected item is not a regular file."
        case let .unreadable(message):
            return "The schematic could not be read: \(message)"
        case .unsupportedExtension:
            return "LitematicaQL can only preview .litematic files."
        }
    }
}

enum LitematicFile {
    static let fileExtension = "litematic"
    static let maximumCompressedFileSize = 32 * 1_024 * 1_024
    private static let gzipHeader: [UInt8] = [0x1f, 0x8b]

    static func readValidatedData(from url: URL) throws -> Data {
        guard url.pathExtension.lowercased() == fileExtension else {
            throw LitematicFileError.unsupportedExtension
        }

        let hasSecurityScope = url.startAccessingSecurityScopedResource()
        defer {
            if hasSecurityScope {
                url.stopAccessingSecurityScopedResource()
            }
        }

        do {
            let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
            guard values.isRegularFile == true else {
                throw LitematicFileError.notAFile
            }
            if let fileSize = values.fileSize, fileSize > maximumCompressedFileSize {
                throw LitematicFileError.fileTooLarge(maximumBytes: maximumCompressedFileSize)
            }
        } catch let error as LitematicFileError {
            throw error
        } catch {
            throw LitematicFileError.unreadable(error.localizedDescription)
        }

        let data: Data
        do {
            data = try Data(contentsOf: url, options: .mappedIfSafe)
        } catch {
            throw LitematicFileError.unreadable(error.localizedDescription)
        }

        guard !data.isEmpty else {
            throw LitematicFileError.emptyFile
        }
        guard data.count <= maximumCompressedFileSize else {
            throw LitematicFileError.fileTooLarge(maximumBytes: maximumCompressedFileSize)
        }
        guard data.count >= gzipHeader.count,
              Array(data.prefix(gzipHeader.count)) == gzipHeader else {
            throw LitematicFileError.invalidHeader
        }

        return data
    }
}
