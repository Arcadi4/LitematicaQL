import Foundation
import Testing
@testable import LitematicaQLCore

@Suite("Saved texture packs")
@MainActor
struct TexturePackLibraryTests {
    @Test("Keeps multiple archives and restores the selected pack")
    func restoresLibrary() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        try library.select(first.id)

        var restored = try fixture.open()
        #expect(restored.packs == [first, second])
        #expect(restored.selectedID == first.id)
        #expect(try Data(contentsOf: restored.fileURL(for: first.id)) == Data([1, 2, 3]))
        #expect(try Data(contentsOf: restored.fileURL(for: second.id)) == Data([1, 2, 3]))

        try restored.select(second.id)
        #expect(try fixture.open().selectedID == second.id)
        try restored.select(nil)
        #expect(try fixture.open().selectedID == nil)
        #expect(try fixture.open().packs == [first, second])
    }

    @Test("Removing an inactive pack keeps the active pack and its archive")
    func removesInactivePack() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        try library.select(second.id)

        try library.remove(first.id)

        let restored = try fixture.open()
        #expect(restored.packs == [second])
        #expect(restored.selectedID == second.id)
        #expect(!FileManager.default.fileExists(atPath: library.fileURL(for: first.id).path))
        #expect(try Data(contentsOf: library.fileURL(for: second.id)) == Data([1, 2, 3]))
    }

    @Test("Removing the active pack persists Vanilla and keeps other packs")
    func removesActivePack() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        try library.select(first.id)

        try library.remove(first.id)

        let restored = try fixture.open()
        #expect(restored.packs == [second])
        #expect(restored.selectedID == nil)
        #expect(!FileManager.default.fileExists(atPath: library.fileURL(for: first.id).path))
    }

    @Test("Same-named imports remain distinct and individually removable")
    func distinguishesSameNamedPacks() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "Pack", to: &library)
        let second = try fixture.addArchive(named: "Pack", to: &library)
        #expect(first.id != second.id)
        #expect(first.name != second.name)

        try library.remove(second.id)

        #expect(try fixture.open().packs == [first])
        #expect(FileManager.default.fileExists(atPath: library.fileURL(for: first.id).path))
    }

    @Test("Missing archives can still be removed from the library")
    func removesMissingArchive() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let pack = try fixture.addArchive(named: "Missing", to: &library)
        try library.select(pack.id)
        try FileManager.default.removeItem(at: library.fileURL(for: pack.id))

        try library.remove(pack.id)

        #expect(try fixture.open().packs.isEmpty)
        #expect(try fixture.open().selectedID == nil)
    }

    @Test("Corrupt saved data is reported without overwriting it")
    func preservesUnreadableLibrary() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let data = Data("invalid".utf8)
        fixture.defaults.set(data, forKey: "texturePackLibrary")

        #expect(throws: (any Error).self) { try fixture.open() }

        #expect(fixture.defaults.data(forKey: "texturePackLibrary") == data)
    }

    @Test("Publishes the selected archive for the Quick Look extension")
    func publishesSelectionFile() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try TexturePackLibrary(
            defaults: fixture.defaults, directory: fixture.directory,
            publishesSelection: true, selectionDirectory: fixture.directory
        )

        let pack = try fixture.addArchive(named: "Shared", to: &library)
        try library.select(pack.id)
        let selectionURL = fixture.directory.appendingPathComponent("Selection.json")
        #expect(FileManager.default.fileExists(atPath: selectionURL.path))
        #expect(TexturePackLibrary.sharedSelectedFileURL(directory: fixture.directory) == library.fileURL(for: pack.id))

        try library.select(nil)
        #expect(TexturePackLibrary.sharedSelectedFileURL(directory: fixture.directory) == nil)

        try library.select(pack.id)
        try FileManager.default.removeItem(at: library.fileURL(for: pack.id))
        #expect(TexturePackLibrary.sharedSelectedFileURL(directory: fixture.directory) == nil)
        #expect(try fixture.open().selectedID == pack.id)
    }

    @Test("Migrates the legacy library into the shared container")
    func migratesLegacyLibrary() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let legacyDefaults = try #require(
            UserDefaults(suiteName: "\(fixture.suiteName).legacy")
        )
        let legacyDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("legacy-\(UUID().uuidString)", isDirectory: true)
        let destinationDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("shared-\(UUID().uuidString)", isDirectory: true)
        defer { legacyDefaults.removePersistentDomain(forName: "\(fixture.suiteName).legacy") }
        try FileManager.default.createDirectory(at: legacyDirectory, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: destinationDirectory, withIntermediateDirectories: true)

        var legacy = try TexturePackLibrary(
            defaults: legacyDefaults, directory: legacyDirectory,
            selectionDirectory: legacyDirectory
        )
        let pack = try fixture.addArchive(named: "Legacy", to: &legacy)
        try legacy.select(pack.id)
        let legacyArchive = try Data(contentsOf: legacy.fileURL(for: pack.id))
        #expect(TexturePackLibrary.migrateLegacyState(
            from: legacyDefaults, legacyDirectory: legacyDirectory,
            to: fixture.defaults, directory: destinationDirectory,
            publishingSelectionIn: destinationDirectory
        ))

        var migrated = try TexturePackLibrary(
            defaults: fixture.defaults, directory: destinationDirectory,
            selectionDirectory: destinationDirectory
        )
        defer {
            try? FileManager.default.removeItem(at: legacyDirectory)
            try? FileManager.default.removeItem(at: destinationDirectory)
        }
        #expect(migrated.packs == [pack])
        #expect(migrated.selectedID == pack.id)
        #expect(try Data(contentsOf: migrated.fileURL(for: pack.id)) == legacyArchive)
        #expect(!FileManager.default.fileExists(atPath: legacyDirectory.path))
        #expect(legacyDefaults.data(forKey: "texturePackLibrary") == nil)

        // A second migration run does nothing. The shared destination is
        // already the source of truth.
        try migrated.select(nil)
        #expect(TexturePackLibrary.migrateLegacyState(
            from: legacyDefaults, legacyDirectory: legacyDirectory,
            to: fixture.defaults, directory: destinationDirectory
        ))
        #expect(try fixture.open().selectedID == nil)
    }

    @Test("A host without legacy state leaves the shared library alone")
    func skipsMigrationWithoutLegacyState() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let legacyDefaults = try #require(
            UserDefaults(suiteName: "\(fixture.suiteName).legacy")
        )
        let legacyDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("legacy-\(UUID().uuidString)", isDirectory: true)
        let destinationDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("shared-\(UUID().uuidString)", isDirectory: true)
        defer { legacyDefaults.removePersistentDomain(forName: "\(fixture.suiteName).legacy") }

        #expect(TexturePackLibrary.migrateLegacyState(
            from: legacyDefaults, legacyDirectory: legacyDirectory,
            to: fixture.defaults, directory: destinationDirectory
        ))

        #expect(try fixture.open().packs.isEmpty)
        #expect(legacyDefaults.data(forKey: "texturePackLibrary") == nil)
        #expect(!FileManager.default.fileExists(atPath: destinationDirectory.path))
    }

    @Test("Existing shared state makes migration preserve the legacy library")
    func preservesLegacyWhenSharedExists() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let legacyDefaults = try #require(
            UserDefaults(suiteName: "\(fixture.suiteName).legacy")
        )
        let legacyDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("legacy-\(UUID().uuidString)", isDirectory: true)
        let destinationDirectory = FileManager.default.temporaryDirectory
            .appendingPathComponent("shared-\(UUID().uuidString)", isDirectory: true)
        defer { legacyDefaults.removePersistentDomain(forName: "\(fixture.suiteName).legacy") }
        try FileManager.default.createDirectory(at: legacyDirectory, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: destinationDirectory, withIntermediateDirectories: true)

        var shared = try TexturePackLibrary(
            defaults: fixture.defaults, directory: destinationDirectory,
            selectionDirectory: destinationDirectory
        )
        _ = try fixture.addArchive(named: "Shared", to: &shared)

        var legacy = try TexturePackLibrary(
            defaults: legacyDefaults, directory: legacyDirectory,
            selectionDirectory: legacyDirectory
        )
        let pack = try fixture.addArchive(named: "Legacy", to: &legacy)
        try legacy.select(pack.id)

        #expect(TexturePackLibrary.migrateLegacyState(
            from: legacyDefaults, legacyDirectory: legacyDirectory,
            to: fixture.defaults, directory: destinationDirectory
        ))

        #expect(try fixture.open().packs == [TexturePackLibrary.Pack(
            id: shared.packs[0].id, name: "Shared"
        )])
        #expect(legacyDefaults.data(forKey: "texturePackLibrary") != nil)
        #expect(FileManager.default.fileExists(atPath: legacyDirectory.path))
    }
}

@MainActor
private struct LibraryFixture {
    let suiteName = "TexturePackLibraryTests.\(UUID().uuidString)"
    let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    let defaults: UserDefaults

    init() throws {
        defaults = try #require(UserDefaults(suiteName: suiteName))
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    func open() throws -> TexturePackLibrary {
        try TexturePackLibrary(defaults: defaults, directory: directory)
    }

    func addArchive(named name: String, to library: inout TexturePackLibrary) throws -> TexturePackLibrary.Pack {
        let pack = TexturePackLibrary.Pack(id: UUID(), name: name)
        try Data([1, 2, 3]).write(to: library.fileURL(for: pack.id))
        return try library.add(pack)
    }

    func cleanUp() {
        defaults.removePersistentDomain(forName: suiteName)
        try? FileManager.default.removeItem(at: directory)
    }
}
