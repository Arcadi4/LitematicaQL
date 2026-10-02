import Foundation
import SwiftUI

/// Keeps an app-owned copy so moving the original ZIP cannot break previews.
@MainActor
final class TexturePackStore: ObservableObject {
    enum Mode: String, CaseIterable {
        case vanilla, custom
    }

    @Published var mode: Mode = .vanilla {
        didSet { UserDefaults.standard.set(mode.rawValue, forKey: "textureRenderingMode") }
    }
    @Published private(set) var name = ""
    @Published private var customPack: NativeResourcePack?
    @Published private(set) var isLoading = false
    @Published var errorMessage: String?

    private struct Selection: Codable, Sendable {
        let id: UUID
        let name: String
    }

    private static let selectionKey = "texturePackSelection"
    private var selection: Selection?

    var hasCustomPack: Bool { selection != nil }
    var resourcePack: NativeResourcePack? { mode == .custom ? customPack : nil }

    init() {
        mode = Mode(rawValue: UserDefaults.standard.string(forKey: "textureRenderingMode") ?? "") ?? .vanilla
        guard let data = UserDefaults.standard.data(forKey: Self.selectionKey),
              let saved = try? JSONDecoder().decode(Selection.self, from: data) else {
            if mode == .custom { mode = .vanilla }
            return
        }
        isLoading = true
        Task {
            defer { isLoading = false }
            do {
                let pack = try await Task.detached(priority: .userInitiated) {
                    let data = try Data(contentsOf: Self.fileURL(for: saved.id), options: .mappedIfSafe)
                    return try NativeResourcePack(data, overlaying: NativeResourcePack.bundled.get())
                }.value
                selection = saved
                name = saved.name
                customPack = pack
            } catch {
                errorMessage = "\(saved.name) could not be restored. The default textures are in use. Choose the pack again."
                UserDefaults.standard.removeObject(forKey: Self.selectionKey)
                if mode == .custom { mode = .vanilla }
            }
        }
    }

    func importPack(from url: URL) async {
        guard !isLoading else { return }
        isLoading = true
        errorMessage = nil
        defer { isLoading = false }
        let next = Selection(id: UUID(), name: url.deletingPathExtension().lastPathComponent)
        do {
            let pack = try await Task.detached(priority: .userInitiated) {
                let access = url.startAccessingSecurityScopedResource()
                defer { if access { url.stopAccessingSecurityScopedResource() } }
                let values = try url.resourceValues(forKeys: [.fileSizeKey, .isRegularFileKey])
                guard values.isRegularFile == true, let size = values.fileSize,
                      size > 0, size <= 256 * 1024 * 1024 else {
                    throw NativeSchematicRefusal(message: "Choose a resource-pack ZIP smaller than 256 MB.")
                }
                let data = try Data(contentsOf: url, options: .mappedIfSafe)
                let pack = try NativeResourcePack(data, overlaying: NativeResourcePack.bundled.get())
                let destination = Self.fileURL(for: next.id)
                try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(),
                                                        withIntermediateDirectories: true)
                try data.write(to: destination, options: .atomic)
                return pack
            }.value
            let previous = selection
            UserDefaults.standard.set(try JSONEncoder().encode(next), forKey: Self.selectionKey)
            selection = next
            name = next.name
            customPack = pack
            mode = .custom
            if let previous { try? FileManager.default.removeItem(at: Self.fileURL(for: previous.id)) }
        } catch {
            errorMessage = error.localizedDescription
        }
    }

    nonisolated private static func fileURL(for id: UUID) -> URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("LitematicaQL/Texture Packs", isDirectory: true)
            .appendingPathComponent(id.uuidString).appendingPathExtension("zip")
    }
}
