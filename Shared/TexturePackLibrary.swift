import Foundation

/// The saved library is independent of which pack is currently parsed for rendering.
///
/// State lives in the app-group container so the Quick Look extension can read
/// the same archives and selection the app manages. Both hosts fall back to
/// their own container when the group is unavailable, which keeps tests and
/// ad-hoc builds working without the entitlement.
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
    /// Plain JSON, because `UserDefaults(suiteName:)` does not resolve group
    /// suites inside every extension runtime even when the group container
    /// itself is accessible.
    struct SelectionHandoff: Codable {
        var selectedID: UUID?
    }

    private static let libraryKey = "texturePackLibrary"
    private let defaults: UserDefaults
    private let publishesSelection: Bool
    /// Where the selection handoff is published; nil resolves the group
    /// container. Tests point it at their own fixtures.
    private let selectionDirectory: URL?
    let directory: URL
    private var saved: SavedLibrary

    /// The library defaults domain, shared through the app-group container when
    /// it is available. The SwiftPM package and tests use standard defaults
    /// because they run outside the sandbox entirely.
    static func sharedDefaults() -> UserDefaults {
        UserDefaults(suiteName: groupIdentifier) ?? .standard
    }

    /// The group container directory, or nil outside an entitled process.
    static func sharedDirectory() -> URL? {
        FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: groupIdentifier
        )?.appendingPathComponent("Texture Packs", isDirectory: true)
    }

    /// Publishes the selected pack id where the Quick Look extension can read
    /// it. Atomic and best-effort: a failed handoff degrades to vanilla Quick
    /// Look rendering while the app itself still works.
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

    /// The archive the Quick Look extension should parse, resolved from the
    /// published selection. Returns nil for vanilla and for selections whose
    /// archive went missing. `directory` overrides the group container, for tests.
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

    /// Register an archive only after it has been validated and copied off-thread.
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

    /// Drops the index entry and deletes the archive. The store deletes through
    /// `removeEntry` instead, so this stays only for tests and direct callers.
    mutating func remove(_ id: UUID) throws {
        let url = fileURL(for: id)
        try removeEntry(id)
        try? FileManager.default.removeItem(at: url)
    }

    /// Drops the index entry without touching the filesystem. The store uses
    /// this on the main actor and deletes the archive off-thread.
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

    /// Drops unreadable saved state so the next launch starts from an empty
    /// library. Archives on disk are left alone; only the index is discarded.
    /// `selectionDirectory` overrides where the published selection is cleared,
    /// for tests; nil means the real group container.
    static func discardSavedState(defaults: UserDefaults = .standard,
                                  selectionDirectory: URL? = nil) {
        defaults.removeObject(forKey: Self.libraryKey)
        writeSelectionFile(nil, directory: selectionDirectory)
    }

    /// Copies the pre-group-container library (per-app `defaults` and
    /// per-app-sandbox archives) into the shared group storage. Every step is
    /// idempotent and failure-tolerant: a failure leaves the source untouched
    /// and changes the destination as far as it could get. Returns true when
    /// the legacy state is gone, either because it moved or never existed.
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
        // A host without legacy state must not write an empty index: that
        // would make the host that does have legacy state skip its migration.
        guard let legacyData = legacyDefaults.data(forKey: libraryKey) else { return true }
        guard let sharedDirectory else { return false }

        var saved = SavedLibrary()
        do {
            saved = try JSONDecoder().decode(SavedLibrary.self, from: legacyData)
        } catch {
            // Unreadable state has nothing to carry over; start from empty.
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
            // Partial migration is retryable: fewer files copied only means
            // fewer to copy next time.
            return false
        }
    }

    /// The library visible to this process after migrating legacy state.
    static func openShared() throws -> TexturePackLibrary {
        _ = migrateLegacyState()
        return try TexturePackLibrary()
    }

    /// The archive of the selected pack, for hosts that parse packs lazily.
    /// Returns nil for vanilla and for selections whose archive went missing.
    func selectedFileURL() -> URL? {
        guard let id = selectedID else { return nil }
        let url = fileURL(for: id)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }
}
