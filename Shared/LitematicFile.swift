import Foundation

enum LitematicFileError: Equatable, LocalizedError {
    case emptyFile
    case fileTooLarge(maximumBytes: Int)
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
        case .notAFile:
            return "The selected item is not a regular file."
        case let .unreadable(message):
            return "The schematic could not be read: \(message)"
        case .unsupportedExtension:
            return "LitematicaQL can preview \(LitematicFile.extensionList) files."
        }
    }
}

enum LitematicFile {
    static let supportedFileExtensions = [
        "litematic",
        "schem",
        "schematic",
        "nbt",
        "snbt",
        "mcstructure",
        "nusn",
        "mca",
    ]
    static let maximumFileSize = 1_024 * 1_024 * 1_024

    // Dotted extension list shown in the unsupported-extension error.
    static let extensionList = supportedFileExtensions
        .map { ".\($0)" }
        .formatted(.list(type: .and))

    static func supports(_ url: URL) -> Bool {
        supportedFileExtensions.contains(url.pathExtension.lowercased())
    }

    // Reads a file that passed the extension, regular-file, and size gates.
    //
    // Content is not validated here. The native bridge owns every format
    // decision, and several supported file formats are uncompressed.
    static func readValidatedData(from url: URL) throws -> Data {
        guard supports(url) else {
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
            if let fileSize = values.fileSize, fileSize > maximumFileSize {
                throw LitematicFileError.fileTooLarge(maximumBytes: maximumFileSize)
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
        guard data.count <= maximumFileSize else {
            throw LitematicFileError.fileTooLarge(maximumBytes: maximumFileSize)
        }

        return data
    }
}
