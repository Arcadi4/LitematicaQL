import Foundation

/// The library state lives in the app-group container so the Quick Look
/// extension reads the same packs and selection as the app. Both hosts fall
/// back to their own container when the group is unavailable. Tests and
/// ad-hoc builds then work without the entitlement.
@MainActor
struct TexturePackLibrary {
    /// Shared between the app and the Quick Look preview extension.
    static let groupIdentifier = "group.moe.arcadia.LitematicaQL"

    struct Pack: Codable, Identifiable, Equatable, Sendable {
        let id: UUID
        let name: String
    }

    private struct SavedLibrary: Codable {
        var packs: [Pack] = []
        var selectedID: UUID?
    }

    /// The selection handoff between the host app and its Quick Look extension.
    /// The file format is JSON because `UserDefaults(suiteName:)` does not
    /// resolve group suites in every extension runtime, even when the group
    /// container is accessible.
    struct SelectionHandoff: Codable {
        var selectedID: UUID?
    }

    private static let libraryKey = "texturePackLibrary"
    private let defaults: UserDefaults
    private let publishesSelection: Bool
    /// Where the selection handoff is published. nil means the group
    /// container. Tests point it at their own fixtures.
    private let selectionDirectory: URL?
    let directory: URL
    private var saved: SavedLibrary

    /// The defaults suite for the library, shared through the app-group
    /// container when it is available. The SwiftPM package and tests run
    /// outside the sandbox and get standard defaults.
    static func sharedDefaults() -> UserDefaults {
        UserDefaults(suiteName: groupIdentifier) ?? .standard
    }

    /// The group container directory, or nil outside an entitled process.
    static func sharedDirectory() -> URL? {
        FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: groupIdentifier
        )?.appendingPathComponent("Texture Packs", isDirectory: true)
    }

    /// Publishes the selected pack id where the Quick Look extension reads it.
    /// Atomic and best-effort: a failed write is not fatal. The app keeps
    /// working, and the extension renders without the selected pack.
    static func writeSelectionFile(_ id: UUID?, directory: URL? = nil) {
        guard let directory = directory ?? sharedDirectory() else { return }
        let url = directory.appendingPathComponent("Selection.json")
        do {
            let data = try JSONEncoder().encode(SelectionHandoff(selectedID: id))
            try data.write(to: url, options: .atomic)
        } catch {
            // The handoff never blocks the app's own library updates.
        }
    }

    /// The archive for the Quick Look extension to parse, resolved from the
    /// published selection. Returns nil when no pack is selected and when the
    /// selected archive does not exist. `directory` overrides the group
    /// container, for tests.
    static func sharedSelectedFileURL(directory source: URL? = nil) -> URL? {
        guard let directory = source ?? sharedDirectory() else { return nil }
        let url = directory.appendingPathComponent("Selection.json")
        guard let data = try? Data(contentsOf: url),
              let selection = try? JSONDecoder().decode(SelectionHandoff.self, from: data),
              let id = selection.selectedID else { return nil }
        let archive = directory
            .appendingPathComponent(id.uuidString)
            .appendingPathExtension("zip")
        return FileManager.default.fileExists(atPath: archive.path) ? archive : nil
    }

    init(defaults: UserDefaults? = nil, directory: URL? = nil,
         publishesSelection: Bool? = nil, selectionDirectory: URL? = nil) throws {
        let defaults = defaults ?? Self.sharedDefaults()
        self.defaults = defaults
        self.publishesSelection = publishesSelection ?? (directory == nil)
        self.selectionDirectory = selectionDirectory
        self.directory = directory
            ?? Self.sharedDirectory()
            ?? FileManager.default.urls(
                for: .applicationSupportDirectory, in: .userDomainMask
            )[0].appendingPathComponent("LitematicaQL/Texture Packs", isDirectory: true)

        if let data = defaults.data(forKey: Self.libraryKey) {
            saved = try JSONDecoder().decode(SavedLibrary.self, from: data)
        } else {
            saved = SavedLibrary()
        }
    }

    var packs: [Pack] { saved.packs }
    var selectedID: UUID? { saved.selectedID }

    func fileURL(for id: UUID) -> URL {
        directory.appendingPathComponent(id.uuidString).appendingPathExtension("zip")
    }

    /// Register an archive only after the caller validates it and copies it
    /// off-thread.
    @discardableResult
    mutating func add(_ pack: Pack) throws -> Pack {
        var name = pack.name
        var suffix = 2
        while packs.contains(where: { $0.name.localizedCaseInsensitiveCompare(name) == .orderedSame }) {
            name = "\(pack.name) (\(suffix))"
            suffix += 1
        }
        let entry = Pack(id: pack.id, name: name)
        var next = saved
        next.packs.append(entry)
        try persist(next)
        saved = next
        return entry
    }

    mutating func select(_ id: UUID?) throws {
        guard id == nil || packs.contains(where: { $0.id == id }) else { return }
        var next = saved
        next.selectedID = id
        try persist(next)
        saved = next
    }

    /// Removes the index entry and deletes the archive in this call. The
    /// store uses `removeEntry`, so this method is for tests and direct
    /// callers.
    mutating func remove(_ id: UUID) throws {
        let url = fileURL(for: id)
        try removeEntry(id)
        try? FileManager.default.removeItem(at: url)
    }

    /// Removes the index entry without touching the filesystem. The store
    /// calls this on the main actor and deletes the archive off-thread.
    mutating func removeEntry(_ id: UUID) throws {
        guard packs.contains(where: { $0.id == id }) else { return }
        var next = saved
        next.packs.removeAll { $0.id == id }
        if next.selectedID == id { next.selectedID = nil }
        try persist(next)
        saved = next
    }

    private func persist(_ value: SavedLibrary) throws {
        defaults.set(try JSONEncoder().encode(value), forKey: Self.libraryKey)
        if publishesSelection {
            Self.writeSelectionFile(value.selectedID, directory: selectionDirectory ?? Self.sharedDirectory())
        }
    }

    /// Removes saved state that the library cannot decode, and clears the
    /// published selection, so the next launch starts from an empty library.
    /// Archives on disk are not deleted. `selectionDirectory` overrides where
    /// the selection is cleared, for tests. nil means the real group container.
    static func discardSavedState(defaults: UserDefaults = .standard,
                                  selectionDirectory: URL? = nil) {
        defaults.removeObject(forKey: Self.libraryKey)
        writeSelectionFile(nil, directory: selectionDirectory)
    }

    /// Copies the legacy state (the per-app `defaults` index and the
    /// per-app-sandbox archives) into the shared group storage. Every step is
    /// idempotent. A failure leaves the source unchanged and the destination
    /// partly updated. Returns true when the legacy state is gone: it moved,
    /// or it never existed.
    static func migrateLegacyState() -> Bool {
        migrateLegacyState(
            from: .standard,
            legacyDirectory: FileManager.default.urls(
                for: .applicationSupportDirectory, in: .userDomainMask
            )[0].appendingPathComponent("LitematicaQL/Texture Packs", isDirectory: true),
            to: sharedDefaults(),
            directory: sharedDirectory()
        )
    }

    static func migrateLegacyState(
        from legacyDefaults: UserDefaults,
        legacyDirectory: URL,
        to sharedDefaults: UserDefaults,
        directory sharedDirectory: URL?,
        publishingSelectionIn selectionDirectory: URL? = nil
    ) -> Bool {
        guard sharedDefaults.data(forKey: libraryKey) == nil else { return true }
        // A host without legacy state must not write an empty index. An empty
        // index makes the host with legacy state skip its migration.
        guard let legacyData = legacyDefaults.data(forKey: libraryKey) else { return true }
        guard let sharedDirectory else { return false }

        var saved = SavedLibrary()
        do {
            saved = try JSONDecoder().decode(SavedLibrary.self, from: legacyData)
        } catch {
            // Unreadable legacy state has nothing to copy, so the migration
            // writes an empty index.
        }

        do {
            try FileManager.default.createDirectory(
                at: sharedDirectory, withIntermediateDirectories: true
            )
            for pack in saved.packs {
                let legacyURL = legacyDirectory
                    .appendingPathComponent(pack.id.uuidString)
                    .appendingPathExtension("zip")
                guard FileManager.default.fileExists(atPath: legacyURL.path) else { continue }
                let destination = sharedDirectory
                    .appendingPathComponent(pack.id.uuidString)
                    .appendingPathExtension("zip")
                try? FileManager.default.removeItem(at: destination)
                try FileManager.default.copyItem(at: legacyURL, to: destination)
            }
            sharedDefaults.set(try JSONEncoder().encode(saved), forKey: libraryKey)
            writeSelectionFile(saved.selectedID, directory: selectionDirectory ?? sharedDirectory)
            legacyDefaults.removeObject(forKey: libraryKey)
            try? FileManager.default.removeItem(at: legacyDirectory)
            return true
        } catch {
            // A partial migration is safe to retry. The next call copies every
            // archive again and removes the legacy state only after success.
            return false
        }
    }

    /// The library visible to this process after migrating legacy state.
    static func openShared() throws -> TexturePackLibrary {
        _ = migrateLegacyState()
        return try TexturePackLibrary()
    }

    /// The archive of the selected pack, for hosts that parse packs lazily.
    /// Returns nil when no pack is selected and when the archive does not
    /// exist.
    func selectedFileURL() -> URL? {
        guard let id = selectedID else { return nil }
        let url = fileURL(for: id)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }
}
