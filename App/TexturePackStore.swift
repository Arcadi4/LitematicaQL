import Foundation
import SwiftUI

@MainActor
final class TexturePackStore: ObservableObject {
    @Published private var library: TexturePackLibrary?
    @Published private(set) var resourcePack: NativeResourcePack?
    @Published private(set) var loadingID: UUID?
    @Published private(set) var isImporting = false
    @Published var errorMessage: String?

    /// Identifies the newest user intent. Loads and imports that started under an
    /// older intent are dropped instead of overwriting a later choice.
    private var revision = UUID()
    private var packLoadTask: Task<Void, Never>?

    var packs: [TexturePackLibrary.Pack] { library?.packs ?? [] }
    var selectedID: UUID? { library?.selectedID }
    var isAvailable: Bool { library != nil }

    init() {
        do {
            library = try TexturePackLibrary()
            if let id = selectedID { selectPack(id) }
        } catch {
            errorMessage = "The saved texture packs could not be restored. \(error.localizedDescription)"
        }
    }

    func selectPack(_ id: UUID?) {
        guard let library, id == nil || library.packs.contains(where: { $0.id == id }) else { return }
        // Re-picking the pack already on screen would rebuild identical geometry.
        guard id != selectedID || loadingID != nil || resourcePack == nil else { return }
        revision = UUID()
        errorMessage = nil
        packLoadTask?.cancel()

        guard let id else {
            do {
                try self.library?.select(nil)
                resourcePack = nil
            } catch { errorMessage = error.localizedDescription }
            return
        }

        loadingID = id
        let revision = revision
        packLoadTask = Task { [weak self] in await self?.load(id, revision: revision) }
    }

    /// Parses one saved archive off-thread and publishes it only while it is still
    /// the newest choice.
    private func load(_ id: UUID, revision: UUID) async {
        guard let entry = library?.packs.first(where: { $0.id == id }),
              let url = library?.fileURL(for: id) else { return }
        do {
            let pack = try await Task.detached(priority: .userInitiated) {
                let data = try Data(contentsOf: url, options: .mappedIfSafe)
                return try NativeResourcePack(data, overlaying: NativeResourcePack.bundled.get())
            }.value
            guard revision == self.revision else { return }
            try self.library?.select(id)
            resourcePack = pack
            loadingID = nil
        } catch is CancellationError {
            // A newer choice already owns the selection and its progress state.
        } catch {
            guard revision == self.revision else { return }
            loadingID = nil
            // Keep the saved selection so the entry stays available for retry, but
            // fall back to vanilla so the menu never claims a pack we cannot show.
            if resourcePack == nil { try? self.library?.select(nil) }
            errorMessage = "\(entry.name) could not be loaded. \(error.localizedDescription)"
        }
    }

    func importPacks(from urls: [URL]) async {
        guard !isImporting, let library else { return }
        isImporting = true
        errorMessage = nil
        defer { isImporting = false }
        let revision = revision
        var failures: [String] = []
        var lastImported: (TexturePackLibrary.Pack, NativeResourcePack)?

        for url in urls {
            let entry = TexturePackLibrary.Pack(id: UUID(), name: url.deletingPathExtension().lastPathComponent)
            let destination = library.fileURL(for: entry.id)
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
                    try FileManager.default.createDirectory(at: destination.deletingLastPathComponent(),
                                                            withIntermediateDirectories: true)
                    try data.write(to: destination, options: .atomic)
                    return pack
                }.value
                if let added = try self.library?.add(entry) {
                    lastImported = (added, pack)
                }
            } catch {
                try? FileManager.default.removeItem(at: destination)
                failures.append("\(entry.name): \(error.localizedDescription)")
            }
        }

        // Choosing or removing a pack during import takes precedence over auto-selection.
        if revision == self.revision, let (entry, pack) = lastImported {
            do {
                try self.library?.select(entry.id)
                packLoadTask?.cancel()
                self.revision = UUID()
                loadingID = nil
                resourcePack = pack
            } catch { failures.append(error.localizedDescription) }
        }
        if !failures.isEmpty { errorMessage = failures.joined(separator: "\n\n") }
    }

    func removePack(_ id: UUID) {
        let removesActivePack = selectedID == id
        do {
            try library?.remove(id)
            revision = UUID()
            if removesActivePack || loadingID == id {
                packLoadTask?.cancel()
                loadingID = nil
            }
            if removesActivePack { resourcePack = nil }
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
