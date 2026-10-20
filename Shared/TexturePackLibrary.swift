import Foundation

/// The library state lives in the app-group container so the Quick Look
/// extension reads the same packs and stack as the app. Both hosts fall
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

    /// The persisted priority order is highest first. Composition reverses it.
    private struct SavedLibrary: Codable, Equatable {
        var packs: [Pack] = []
        var enabledIDs: [UUID] = []

        private enum CodingKeys: String, CodingKey {
            case packs
            case enabledIDs
            case selectedID
        }

        init() {}

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            packs = try container.decodeIfPresent([Pack].self, forKey: .packs) ?? []
            if let enabledIDs = try container.decodeIfPresent([UUID].self, forKey: .enabledIDs) {
                self.enabledIDs = enabledIDs
            } else if let selectedID = try container.decodeIfPresent(UUID.self, forKey: .selectedID) {
                self.enabledIDs = [selectedID]
            }
        }

        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(packs, forKey: .packs)
            try container.encode(enabledIDs, forKey: .enabledIDs)
        }
    }

    /// The stack handoff between the host app and its Quick Look extension.
    /// The file format is JSON because `UserDefaults(suiteName:)` does not
    /// resolve group suites in every extension runtime, even when the group
    /// container is accessible. Entries are highest priority first.
    struct SelectionHandoff: Codable {
        struct Entry: Codable, Equatable, Sendable {
            let id: UUID
            let name: String
        }

        var enabled: [Entry]

        private enum CodingKeys: String, CodingKey {
            case enabled
            case selectedID
        }

        init(enabled: [Entry]) {
            self.enabled = enabled
        }

        init(from decoder: Decoder) throws {
            let container = try decoder.container(keyedBy: CodingKeys.self)
            if let enabled = try container.decodeIfPresent([Entry].self, forKey: .enabled) {
                self.enabled = enabled
            } else if let selectedID = try container.decodeIfPresent(UUID.self, forKey: .selectedID) {
                self.enabled = [Entry(id: selectedID, name: "Selected texture pack")]
            } else {
                self.enabled = []
            }
        }

        func encode(to encoder: Encoder) throws {
            var container = encoder.container(keyedBy: CodingKeys.self)
            try container.encode(enabled, forKey: .enabled)
        }
    }

    struct SharedPack: Equatable, Sendable {
        let id: UUID
        let name: String
        let url: URL
    }

    /// `packs` is lowest priority first, ready for composition. `missing`
    /// names enabled packs whose archive is absent. `unreadable` marks a
    /// handoff file that exists but cannot be decoded.
    struct SharedSelection: Equatable, Sendable {
        let packs: [SharedPack]
        let missing: [SharedPack]
        let unreadable: Bool
    }

    private static let libraryKey = "texturePackLibrary"
    private let defaults: UserDefaults
    private let publishesSelection: Bool
    /// Where the stack handoff is published. nil means the group container.
    /// Tests point it at their own fixtures.
    private let selectionDirectory: URL?
    private let directory: URL
    private var saved: SavedLibrary
    private var lastPersistedEnabledIDs: [UUID]
    private(set) var needsMigration: Bool

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

    /// Publishes the enabled stack where the Quick Look extension reads it.
    /// Atomic and best-effort: a failed write is not fatal. The app keeps
    /// working, and the extension renders with the previous or bundled stack.
    static func writeSelectionFile(_ entries: [SelectionHandoff.Entry],
                                   directory: URL? = nil) {
        guard let directory = directory ?? sharedDirectory() else { return }
        let url = directory.appendingPathComponent("Selection.json")
        do {
            let data = try JSONEncoder().encode(SelectionHandoff(enabled: entries))
            try data.write(to: url, options: .atomic)
        } catch {
            // The handoff never blocks the app's own library updates.
        }
    }

    /// The stack for the Quick Look extension. Returns nil when no pack is
    /// enabled. Missing archives and an unreadable handoff stay visible to
    /// the caller, so the preview can state why it fell back.
    static func sharedSelection(directory source: URL? = nil) -> SharedSelection? {
        guard let directory = source ?? sharedDirectory() else { return nil }
        let url = directory.appendingPathComponent("Selection.json")
        guard FileManager.default.fileExists(atPath: url.path) else { return nil }
        guard let data = try? Data(contentsOf: url),
              let handoff = try? JSONDecoder().decode(SelectionHandoff.self, from: data) else {
            return SharedSelection(packs: [], missing: [], unreadable: true)
        }
        guard !handoff.enabled.isEmpty else { return nil }

        var packs: [SharedPack] = []
        var missing: [SharedPack] = []
        var seen: Set<UUID> = []
        for entry in handoff.enabled.reversed() {
            guard seen.insert(entry.id).inserted else { continue }
            let archive = directory
                .appendingPathComponent(entry.id.uuidString)
                .appendingPathExtension("zip")
            let pack = SharedPack(id: entry.id, name: entry.name, url: archive)
            if FileManager.default.fileExists(atPath: archive.path) {
                packs.append(pack)
            } else {
                missing.append(pack)
            }
        }
        return SharedSelection(packs: packs, missing: missing, unreadable: false)
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
            let decoded = try JSONDecoder().decode(SavedLibrary.self, from: data)
            saved = Self.normalized(decoded)
            needsMigration = Self.hasLegacySelection(data) || saved != decoded
        } else {
            saved = SavedLibrary()
            needsMigration = false
        }
        lastPersistedEnabledIDs = saved.enabledIDs
    }

    var packs: [Pack] { saved.packs }
    var enabledIDs: [UUID] { saved.enabledIDs }

    var enabledPacks: [Pack] {
        saved.enabledIDs.compactMap { id in
            saved.packs.first { $0.id == id }
        }
    }

    var disabledPacks: [Pack] {
        saved.packs.filter { !saved.enabledIDs.contains($0.id) }
    }

    func isEnabled(_ id: UUID) -> Bool {
        saved.enabledIDs.contains(id)
    }

    func fileURL(for id: UUID) -> URL {
        directory.appendingPathComponent(id.uuidString).appendingPathExtension("zip")
    }

    /// Register an archive only after the caller validates it and copies it
    /// off-thread. The enabled stack does not change.
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

    /// Replaces the enabled stack in memory. The caller persists it after the
    /// composed stack loads.
    mutating func setEnabledIDs(_ ids: [UUID]) {
        saved.enabledIDs = Self.validEnabledIDs(ids, packs: saved.packs)
    }

    /// Adds a pack at the requested priority index. Zero is highest priority.
    mutating func enable(_ id: UUID, at index: Int = 0) {
        guard saved.packs.contains(where: { $0.id == id }),
              !saved.enabledIDs.contains(id) else { return }
        saved.enabledIDs.insert(id, at: min(max(index, 0), saved.enabledIDs.count))
    }

    mutating func disable(_ id: UUID) {
        saved.enabledIDs.removeAll { $0 == id }
    }

    mutating func moveEnabled(_ id: UUID, by offset: Int) {
        guard let index = saved.enabledIDs.firstIndex(of: id) else { return }
        let target = min(max(index + offset, 0), saved.enabledIDs.count - 1)
        guard target != index else { return }
        let moved = saved.enabledIDs.remove(at: index)
        saved.enabledIDs.insert(moved, at: target)
    }

    mutating func moveEnabled(fromOffsets source: IndexSet, toOffset destination: Int) {
        var ids = saved.enabledIDs
        guard source.allSatisfy({ ids.indices.contains($0) }) else { return }
        let moving = source.sorted().map { ids[$0] }
        var target = destination
        for index in source.sorted(by: >) {
            ids.remove(at: index)
            if index < destination { target -= 1 }
        }
        ids.insert(contentsOf: moving, at: min(max(target, 0), ids.count))
        saved.enabledIDs = ids
    }

    /// Writes the current enabled stack and its handoff. Call it only after a
    /// complete stack loads.
    mutating func commit() throws {
        try persist(saved)
    }

    /// Returns the in-memory stack to the last stack on disk.
    mutating func revertEnabled() {
        saved.enabledIDs = Self.validEnabledIDs(lastPersistedEnabledIDs, packs: saved.packs)
    }

    /// Rewrites a decoded legacy single selection in the enabled-stack format.
    mutating func persistMigrationIfNeeded() throws {
        guard needsMigration else { return }
        try persist(saved)
        needsMigration = false
    }

    /// Removes the index entry without touching the filesystem. The store
    /// calls this on the main actor and deletes the archive off-thread.
    mutating func removeEntry(_ id: UUID) throws {
        guard packs.contains(where: { $0.id == id }) else { return }
        var next = saved
        next.packs.removeAll { $0.id == id }
        next.enabledIDs.removeAll { $0 == id }
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

    private mutating func persist(_ value: SavedLibrary) throws {
        defaults.set(try JSONEncoder().encode(value), forKey: Self.libraryKey)
        if publishesSelection {
            Self.writeSelectionFile(
                Self.handoffEntries(for: value),
                directory: selectionDirectory ?? Self.sharedDirectory()
            )
        }
        lastPersistedEnabledIDs = value.enabledIDs
        needsMigration = false
    }

    private static func handoffEntries(for value: SavedLibrary) -> [SelectionHandoff.Entry] {
        value.enabledIDs.compactMap { id in
            value.packs.first { $0.id == id }.map {
                SelectionHandoff.Entry(id: $0.id, name: $0.name)
            }
        }
    }

    private static func normalized(_ value: SavedLibrary) -> SavedLibrary {
        var result = value
        result.enabledIDs = validEnabledIDs(value.enabledIDs, packs: value.packs)
        return result
    }

    private static func validEnabledIDs(_ ids: [UUID], packs: [Pack]) -> [UUID] {
        var seen: Set<UUID> = []
        return ids.filter { id in
            packs.contains { $0.id == id } && seen.insert(id).inserted
        }
    }

    private static func hasLegacySelection(_ data: Data) -> Bool {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return false
        }
        return object["enabledIDs"] == nil && object["selectedID"] != nil
    }

    /// Removes saved state that the library cannot decode, and clears the
    /// published stack, so the next launch starts from an empty library.
    /// Archives on disk are not deleted. `selectionDirectory` overrides where
    /// the handoff is cleared, for tests. nil means the real group container.
    static func discardSavedState(defaults: UserDefaults = .standard,
                                  selectionDirectory: URL? = nil) {
        defaults.removeObject(forKey: Self.libraryKey)
        writeSelectionFile([], directory: selectionDirectory)
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
            saved = normalized(try JSONDecoder().decode(SavedLibrary.self, from: legacyData))
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
            writeSelectionFile(handoffEntries(for: saved),
                               directory: selectionDirectory ?? sharedDirectory)
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
        var library = try TexturePackLibrary()
        try? library.persistMigrationIfNeeded()
        return library
    }
}
