import Foundation
import SwiftUI

@MainActor
final class TexturePackStore: ObservableObject {
    @Published private var library: TexturePackLibrary?
    @Published private(set) var resourcePack: NativeResourcePack?
    @Published private(set) var loadingID: UUID?
    @Published private(set) var isImporting = false
    @Published var errorMessage: String?

    private var loadGeneration = UUID()
    private var selectionRevision = UUID()
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
        selectionRevision = UUID()
        loadGeneration = UUID()
        loadingID = id
        errorMessage = nil

        guard id != nil else {
            do {
                try self.library?.select(nil)
                resourcePack = nil
            } catch { errorMessage = error.localizedDescription }
            return
        }

        // Coalesce rapid choices so only one saved archive is parsed at a time.
        guard packLoadTask == nil else { return }
        packLoadTask = Task {
            defer { packLoadTask = nil }
            while let id = loadingID, let entry = self.library?.packs.first(where: { $0.id == id }),
                  let url = self.library?.fileURL(for: id) {
                let generation = loadGeneration
                do {
                    let pack = try await Task.detached(priority: .userInitiated) {
                        let data = try Data(contentsOf: url, options: .mappedIfSafe)
                        return try NativeResourcePack(data, overlaying: NativeResourcePack.bundled.get())
                    }.value
                    guard generation == loadGeneration else { continue }
                    try self.library?.select(id)
                    resourcePack = pack
                    loadingID = nil
                } catch {
                    guard generation == loadGeneration else { continue }
                    loadingID = nil
                    // A failed restore keeps the entry available for removal or retry.
                    if resourcePack == nil { try? self.library?.select(nil) }
                    errorMessage = "\(entry.name) could not be loaded. \(error.localizedDescription)"
                }
            }
        }
    }

    func importPacks(from urls: [URL]) async {
        guard !isImporting, let library else { return }
        isImporting = true
        errorMessage = nil
        defer { isImporting = false }
        let revision = selectionRevision
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
        if revision == selectionRevision, let (entry, pack) = lastImported {
            do {
                try self.library?.select(entry.id)
                loadGeneration = UUID()
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
            selectionRevision = UUID()
            if removesActivePack || loadingID == id {
                loadGeneration = UUID()
                loadingID = nil
            }
            if removesActivePack { resourcePack = nil }
        } catch {
            errorMessage = error.localizedDescription
        }
    }
}
