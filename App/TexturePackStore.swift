import Foundation
import SwiftUI

@MainActor
final class TexturePackStore: ObservableObject {
    @Published private var library: TexturePackLibrary?
    @Published private(set) var resourcePack: NativeResourcePack?
    @Published private(set) var loadingID: UUID?
    @Published private(set) var isImporting = false
    /// One presentation surface for every pack failure.
    struct Alert: Equatable {
        let title: String
        let message: String
    }

    @Published private(set) var alert: Alert?

    /// The pack whose textures are on screen, or the one being parsed. Nothing
    /// loaded means nothing claimed, even while a saved choice is still pending.
    var activeID: UUID? { loadingID ?? (resourcePack == nil ? nil : selectedID) }

    /// Loads started under an older intent are dropped rather than applied late.
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
            alert = Alert(title: "Unable to Open the Texture Pack Library",
                          message: "The saved texture packs could not be restored. \(error.localizedDescription)")
        }
    }

    /// Cancelling the panel is not a failure worth an alert.
    func reportImportFailure(_ error: any Error) {
        let failure = error as NSError
        guard failure.domain != NSCocoaErrorDomain || failure.code != NSUserCancelledError else { return }
        alert = Alert(title: "Unable to Add Texture Packs", message: error.localizedDescription)
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
            saveSelection(nil)
            resourcePack = nil
            loadingID = nil
            return
        }

        loadingID = id
        let snapshot = revision
        packLoadTask = Task { [weak self] in await self?.load(id, revision: snapshot) }
    }

    /// Parses one saved archive off-thread while it is still the newest choice.
    private func load(_ id: UUID, revision: UUID) async {
        guard let entry = library?.packs.first(where: { $0.id == id }),
              let url = library?.fileURL(for: id) else { return }
        do {
            let pack = try await Task.detached(priority: .userInitiated) {
                let data = try Data(contentsOf: url, options: .mappedIfSafe)
                return try NativeResourcePack(data, overlaying: NativeResourcePack.bundled.get())
            }.value
            guard revision == self.revision else { return }
            saveSelection(id)
            resourcePack = pack
            loadingID = nil
        } catch is CancellationError {
            // A newer choice owns the selection and its progress state.
        } catch {
            guard revision == self.revision else { return }
            loadingID = nil
            alert = Alert(title: "Unable to Load Texture Pack",
                          message: "\(entry.name) could not be loaded. \(error.localizedDescription)")
        }
    }

    /// A failed write still shows the pack, but the choice is not remembered.
    private func saveSelection(_ id: UUID?) {
        do { try self.library?.select(id) } catch {
            alert = Alert(title: "Unable to Change Texture Pack", message: error.localizedDescription)
        }
    }

    func importPacks(from urls: [URL]) async {
        guard !isImporting, ensureLibrary(), let library else { return }
        isImporting = true
        alert = nil
        defer { isImporting = false }
        let snapshot = revision
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
        if snapshot == self.revision, let (entry, pack) = lastImported {
            saveSelection(entry.id)
            packLoadTask?.cancel()
            self.revision = UUID()
            loadingID = nil
            resourcePack = pack
        }
        if !failures.isEmpty {
            alert = Alert(title: "Some Texture Packs Could Not Be Added",
                          message: failures.joined(separator: "\n"))
        }
    }

    func removePack(_ id: UUID) {
        let removesShownPack = selectedID == id
        do {
            try library?.remove(id)
            revision = UUID()
            if removesShownPack || loadingID == id {
                packLoadTask?.cancel()
                loadingID = nil
            }
            if removesShownPack { resourcePack = nil }
        } catch {
            alert = Alert(title: "Unable to Remove Texture Pack", message: error.localizedDescription)
        }
    }

    /// The Add button stays enabled when saved state is unreadable, so a fresh
    /// library is the recovery path instead of a dead-end disabled panel.
    @discardableResult
    private func ensureLibrary() -> Bool {
        if library != nil { return true }
        do {
            library = try TexturePackLibrary()
            return true
        } catch {
            TexturePackLibrary.discardSavedState()
            do {
                library = try TexturePackLibrary()
                return true
            } catch {
                alert = Alert(title: "Unable to Add Texture Packs", message: error.localizedDescription)
                return false
            }
        }
    }
}
