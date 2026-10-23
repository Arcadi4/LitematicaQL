import Foundation
import SwiftUI

@MainActor
final class TexturePackStore: ObservableObject {
    @Published private var library: TexturePackLibrary?
    @Published private(set) var resourcePack: NativeResourcePack?
    @Published private(set) var isLoadingStack = false
    @Published private(set) var isImporting = false
    /// The active problem with the configured stack, shown while previews use
    /// the previous stack or the bundled base.
    @Published private(set) var failureMessage: String?
    /// One presentation surface for every pack failure.
    struct Alert: Equatable {
        let title: String
        let message: String
    }

    @Published private(set) var alert: Alert?

    /// The enabled stack that a load in progress will apply. A newer selection
    /// supersedes it, and the stale load is dropped instead of applied late.
    private var revision = UUID()
    private var packLoadTask: Task<Void, Never>?
    /// The stack that `resourcePack` was built from, in priority order.
    private var loadedEnabledIDs: [UUID] = []

    var packs: [TexturePackLibrary.Pack] { library?.packs ?? [] }
    var enabledPacks: [TexturePackLibrary.Pack] { library?.enabledPacks ?? [] }
    var disabledPacks: [TexturePackLibrary.Pack] { library?.disabledPacks ?? [] }
    private var enabledIDs: [UUID] { library?.enabledIDs ?? [] }
    var isBusy: Bool { isImporting || isLoadingStack }

    func isEnabled(_ id: UUID) -> Bool { library?.isEnabled(id) ?? false }

    init() {
        do {
            library = try TexturePackLibrary.openShared()
            if !enabledIDs.isEmpty {
                applyEnabledStack(enabledIDs, revertOnFailure: false, persistOnSuccess: false)
            }
        } catch {
            alert = Alert(title: "Unable to Open the Texture Pack Library",
                          message: "The saved texture packs could not be restored. \(error.localizedDescription)")
        }
    }

    /// A cancelled file panel is not a failure and does not show an alert.
    func reportImportFailure(_ error: any Error) {
        let failure = error as NSError
        guard failure.domain != NSCocoaErrorDomain || failure.code != NSUserCancelledError else { return }
        alert = Alert(title: "Unable to Add Texture Packs", message: error.localizedDescription)
    }

    func dismissAlert() { alert = nil }

    /// Adds at highest priority or removes from the enabled stack. The change
    /// persists only after the complete stack loads.
    func setEnabled(_ id: UUID, _ enabled: Bool) {
        guard var updated = library, updated.packs.contains(where: { $0.id == id }) else { return }
        if enabled {
            updated.enable(id)
        } else {
            updated.disable(id)
        }
        guard updated.enabledIDs != library?.enabledIDs else { return }
        library = updated
        applyEnabledStack(updated.enabledIDs, revertOnFailure: true, persistOnSuccess: true)
    }

    func moveEnabled(fromOffsets source: IndexSet, toOffset destination: Int) {
        mutateEnabled { $0.moveEnabled(fromOffsets: source, toOffset: destination) }
    }

    func moveEnabled(_ id: UUID, by offset: Int) {
        mutateEnabled { $0.moveEnabled(id, by: offset) }
    }

    private func mutateEnabled(_ mutation: (inout TexturePackLibrary) -> Void) {
        guard var updated = library else { return }
        let previous = updated.enabledIDs
        mutation(&updated)
        guard updated.enabledIDs != previous else { return }
        library = updated
        applyEnabledStack(updated.enabledIDs, revertOnFailure: true, persistOnSuccess: true)
    }

    private func applyEnabledStack(_ ids: [UUID], revertOnFailure: Bool,
                                   persistOnSuccess: Bool) {
        revision = UUID()
        let revision = self.revision
        packLoadTask?.cancel()
        alert = nil

        guard !ids.isEmpty else {
            isLoadingStack = false
            resourcePack = nil
            loadedEnabledIDs = []
            failureMessage = nil
            if persistOnSuccess { commitSelection() }
            return
        }

        guard let library = self.library else { return }
        let packsByID = Dictionary(uniqueKeysWithValues: library.packs.map { ($0.id, $0) })
        let ordered = ids.compactMap { packsByID[$0] }
        guard ordered.count == ids.count else {
            failComposition(message: "A saved texture pack is missing from the library.",
                            revertOnFailure: revertOnFailure)
            return
        }
        let sources = ordered.reversed().map {
            NativeResourcePack.Source(name: $0.name, url: library.fileURL(for: $0.id))
        }

        isLoadingStack = true
        packLoadTask = Task { [weak self] in
            do {
                let pack = try await Task.detached(priority: .userInitiated) {
                    let base = try NativeResourcePack.bundled.get()
                    return try NativeResourcePack.composed(from: sources, over: base)
                }.value
                guard let self, revision == self.revision else { return }
                if persistOnSuccess { self.commitSelection() }
                self.resourcePack = pack
                self.loadedEnabledIDs = ids
                self.isLoadingStack = false
                self.failureMessage = nil
                self.packLoadTask = nil
            } catch {
                guard let self, revision == self.revision else { return }
                self.failComposition(message: error.localizedDescription,
                                     revertOnFailure: revertOnFailure)
            }
        }
    }

    private func failComposition(message: String, revertOnFailure: Bool) {
        isLoadingStack = false
        packLoadTask = nil
        if revertOnFailure {
            library?.revertEnabled()
            if resourcePack != nil,
               let reverted = self.library?.enabledIDs,
               reverted == loadedEnabledIDs {
                failureMessage = "The previous texture pack stack is still active. \(message)"
            } else {
                resourcePack = nil
                loadedEnabledIDs = []
                failureMessage = "Previews use bundled vanilla resources. \(message)"
            }
        } else {
            resourcePack = nil
            loadedEnabledIDs = []
            failureMessage = "Previews use bundled vanilla resources. \(message)"
        }
        alert = Alert(title: "Unable to Apply Texture Pack Stack",
                      message: failureMessage ?? message)
    }

    private func commitSelection() {
        do {
            try library?.commit()
        } catch {
            alert = Alert(title: "Unable to Save Texture Packs",
                          message: error.localizedDescription)
        }
    }

    func importPacks(from urls: [URL]) async {
        guard !isImporting, ensureLibrary(), let library = self.library else { return }
        isImporting = true
        alert = nil
        defer { isImporting = false }
        let snapshot = revision
        var failures: [String] = []
        var imported: [TexturePackLibrary.Pack] = []

        for url in urls {
            let entry = TexturePackLibrary.Pack(
                id: UUID(),
                name: url.deletingPathExtension().lastPathComponent
            )
            let destination = library.fileURL(for: entry.id)
            do {
                try await Task.detached(priority: .userInitiated) {
                    let access = url.startAccessingSecurityScopedResource()
                    defer { if access { url.stopAccessingSecurityScopedResource() } }
                    // The native layer rejects oversized, empty, and
                    // non-resource-pack archives with the text the user sees.
                    let data = try Data(contentsOf: url, options: .mappedIfSafe)
                    let base = try NativeResourcePack.bundled.get()
                    _ = try NativeResourcePack(
                        layers: [NativeResourcePack.Layer(name: entry.name, data: data)],
                        over: base
                    )
                    try FileManager.default.createDirectory(
                        at: destination.deletingLastPathComponent(),
                        withIntermediateDirectories: true
                    )
                    try data.write(to: destination, options: .atomic)
                }.value
                if let added = try self.library?.add(entry) {
                    imported.append(added)
                }
            } catch {
                try? FileManager.default.removeItem(at: destination)
                if let refusal = error as? NativeSchematicRefusal {
                    failures.append(refusal.message)
                } else {
                    failures.append("\(entry.name): \(error.localizedDescription)")
                }
            }
        }

        // A pack choice or removal during import takes precedence over
        // auto-enabling the imported packs.
        if snapshot == revision, !imported.isEmpty {
            guard var updated = self.library else { return }
            for entry in imported {
                updated.enable(entry.id)
            }
            self.library = updated
            applyEnabledStack(updated.enabledIDs, revertOnFailure: true, persistOnSuccess: true)
        }
        if !failures.isEmpty {
            alert = Alert(title: "Some Texture Packs Could Not Be Added",
                          message: failures.joined(separator: "\n"))
        }
    }

    func removePack(_ id: UUID) {
        guard let library = self.library,
              library.packs.contains(where: { $0.id == id }) else { return }
        let url = library.fileURL(for: id)
        let wasEnabled = library.isEnabled(id)
        do {
            try self.library?.removeEntry(id)
        } catch {
            alert = Alert(title: "Unable to Remove Texture Pack", message: error.localizedDescription)
            return
        }
        if wasEnabled {
            applyEnabledStack(self.library?.enabledIDs ?? [], revertOnFailure: false,
                              persistOnSuccess: true)
        }
        // Deleting the archive can block. The index is updated on the main
        // actor first, so the file removal runs detached.
        Task.detached(priority: .utility) {
            try? FileManager.default.removeItem(at: url)
        }
    }

    /// The Add button stays enabled when the saved library is unreadable.
    /// The recovery path discards the saved state and opens a new library.
    @discardableResult
    private func ensureLibrary() -> Bool {
        if library != nil { return true }
        do {
            library = try TexturePackLibrary.openShared()
            return true
        } catch {
            TexturePackLibrary.discardSavedState(defaults: TexturePackLibrary.sharedDefaults())
            do {
                library = try TexturePackLibrary.openShared()
                return true
            } catch {
                alert = Alert(title: "Unable to Add Texture Packs", message: error.localizedDescription)
                return false
            }
        }
    }
}
