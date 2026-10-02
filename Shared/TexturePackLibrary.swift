import Foundation

/// The saved library is independent of which pack is currently parsed for rendering.
@MainActor
struct TexturePackLibrary {
    struct Pack: Codable, Identifiable, Equatable, Sendable {
        let id: UUID
        let name: String
    }

    private struct SavedLibrary: Codable {
        var packs: [Pack] = []
        var selectedID: UUID?
    }

    private static let libraryKey = "texturePackLibrary"
    private let defaults: UserDefaults
    let directory: URL
    private var saved: SavedLibrary

    var packs: [Pack] { saved.packs }
    var selectedID: UUID? { saved.selectedID }

    init(defaults: UserDefaults = .standard, directory: URL? = nil) throws {
        self.defaults = defaults
        self.directory = directory ?? FileManager.default.urls(
            for: .applicationSupportDirectory, in: .userDomainMask
        )[0].appendingPathComponent("LitematicaQL/Texture Packs", isDirectory: true)

        if let data = defaults.data(forKey: Self.libraryKey) {
            saved = try JSONDecoder().decode(SavedLibrary.self, from: data)
        } else {
            saved = SavedLibrary()
        }
    }

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

    mutating func remove(_ id: UUID) throws {
        guard packs.contains(where: { $0.id == id }) else { return }
        var next = saved
        next.packs.removeAll { $0.id == id }
        if next.selectedID == id { next.selectedID = nil }
        try persist(next)
        do {
            let url = fileURL(for: id)
            if FileManager.default.fileExists(atPath: url.path) {
                try FileManager.default.removeItem(at: url)
            }
        } catch {
            try persist(saved)
            throw error
        }
        saved = next
    }

    private func persist(_ value: SavedLibrary) throws {
        defaults.set(try JSONEncoder().encode(value), forKey: Self.libraryKey)
    }

    /// Drops unreadable saved state so the next launch starts from an empty
    /// library. Archives on disk are left alone; only the index is discarded.
    static func discardSavedState(defaults: UserDefaults = .standard) {
        defaults.removeObject(forKey: Self.libraryKey)
    }
}
