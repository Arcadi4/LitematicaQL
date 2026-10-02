import Foundation
import SwiftUI

@MainActor
final class TexturePackStore: ObservableObject {
    @Published private var library: TexturePackLibrary?
    @Published private(set) var resourcePack: NativeResourcePack?
    @Published private(set) var loadingID: UUID?
    @Published private(set) var isImporting = false
    /// One presentation surface for every pack failure, so each alert can name
    /// the action that actually failed.
    enum Alert: Equatable {
        case libraryUnavailable(String)
        case loadFailed(String)
        case importFailed([String])
        case importUnavailable(String)

        var title: String {
            switch self {
            case .libraryUnavailable: "Unable to Open the Texture Pack Library"
            case .loadFailed: "Unable to Load Texture Pack"
            case .importFailed: "Some Texture Packs Could Not Be Added"
            case .importUnavailable: "Unable to Add Texture Packs"
            }
        }

        var message: String {
            switch self {
            case let .libraryUnavailable(reason), let .loadFailed(reason),
                 let .importUnavailable(reason):
                reason
            case let .importFailed(failures): failures.joined(separator: "\n")
            }
        }
    }

    @Published private(set) var alert: Alert?

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
            alert = .libraryUnavailable(
                "The saved texture packs could not be restored. \(error.localizedDescription)")
        }
    }

    /// Cancelling the open panel is not a failure the user needs to hear about.
    func reportImportFailure(_ error: any Error) {
        let failure = error as NSError
        guard !(failure.domain == NSCocoaErrorDomain && failure.code == NSUserCancelledError) else {
            return
        }
        alert = .importUnavailable(error.localizedDescription)
    }

    func dismissAlert() { alert = nil }

    func selectPack(_ id: UUID?) {
        guard let library, id == nil || library.packs.contains(where: { $0.id == id }) else { return }
        // Re-picking the pack already on screen would rebuild identical geometry.
        guard id != selectedID || loadingID != nil || resourcePack == nil else { return }
        revision = UUID()
        alert = nil
        packLoadTask?.cancel()

        guard let id else {
            do {
                try self.library?.select(nil)
                resourcePack = nil
            } catch { alert = .loadFailed(error.localizedDescription) }
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
            alert = .loadFailed("\(entry.name) could not be loaded. \(error.localizedDescription)")
        }
    }

    func importPacks(from urls: [URL]) async {
        guard !isImporting, let library else { return }
        isImporting = true
        alert = nil
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
                    // The native loader rejects oversized, empty, and
                    // non-resource-pack archives with the text the user sees.
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
        if !failures.isEmpty { alert = .importFailed(failures) }
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
            alert = .loadFailed(error.localizedDescription)
        }
    }
}
