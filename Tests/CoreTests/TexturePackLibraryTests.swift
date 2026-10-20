import Foundation
import Testing
@testable import LitematicaQLCore

@Suite("Saved texture packs")
@MainActor
struct TexturePackLibraryTests {
    @Test("Keeps the enabled stack in priority order")
    func restoresLibrary() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        library.setEnabledIDs([second.id, first.id])
        try library.commit()

        var restored = try fixture.open()
        #expect(restored.packs == [first, second])
        #expect(restored.enabledIDs == [second.id, first.id])
        #expect(restored.enabledPacks == [second, first])
        #expect(restored.disabledPacks.isEmpty)

        restored.disable(second.id)
        restored.enable(second.id)
        #expect(restored.enabledIDs == [second.id, first.id])

        restored.disable(second.id)
        try restored.commit()
        #expect(try fixture.open().enabledIDs == [first.id])
        #expect(try fixture.open().disabledPacks == [second])

        restored.disable(first.id)
        try restored.commit()
        #expect(try fixture.open().enabledIDs.isEmpty)
        #expect(try fixture.open().packs == [first, second])
    }

    @Test("Moves enabled packs between priorities")
    func movesEnabledPriority() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        let third = try fixture.addArchive(named: "Third", to: &library)
        library.setEnabledIDs([first.id, second.id, third.id])

        library.moveEnabled(first.id, by: 2)
        #expect(library.enabledIDs == [second.id, third.id, first.id])

        library.moveEnabled(fromOffsets: IndexSet(integer: 2), toOffset: 0)
        #expect(library.enabledIDs == [first.id, second.id, third.id])

        library.moveEnabled(fromOffsets: IndexSet([0, 1]), toOffset: 3)
        #expect(library.enabledIDs == [third.id, first.id, second.id])

        try library.commit()
        #expect(try fixture.open().enabledIDs == [third.id, first.id, second.id])
    }

    @Test("Removing an inactive pack keeps the enabled stack and its archives")
    func removesInactivePack() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        library.setEnabledIDs([second.id])
        try library.commit()

        try library.remove(first.id)

        let restored = try fixture.open()
        #expect(restored.packs == [second])
        #expect(restored.enabledIDs == [second.id])
        #expect(!FileManager.default.fileExists(atPath: library.fileURL(for: first.id).path))
        #expect(try Data(contentsOf: library.fileURL(for: second.id)) == Data([1, 2, 3]))
    }

    @Test("Removing an enabled pack removes only that pack from the stack")
    func removesEnabledPack() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try fixture.open()
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        library.setEnabledIDs([first.id, second.id])
        try library.commit()

        try library.remove(first.id)

        let restored = try fixture.open()
        #expect(restored.packs == [second])
        #expect(restored.enabledIDs == [second.id])
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
        library.setEnabledIDs([pack.id])
        try library.commit()
        try FileManager.default.removeItem(at: library.fileURL(for: pack.id))

        try library.remove(pack.id)

        #expect(try fixture.open().packs.isEmpty)
        #expect(try fixture.open().enabledIDs.isEmpty)
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

    @Test("Publishes the enabled stack for Quick Look in composition order")
    func publishesSelectionFile() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        var library = try TexturePackLibrary(
            defaults: fixture.defaults, directory: fixture.directory,
            publishesSelection: true, selectionDirectory: fixture.directory
        )
        let first = try fixture.addArchive(named: "First", to: &library)
        let second = try fixture.addArchive(named: "Second", to: &library)
        library.setEnabledIDs([first.id, second.id])
        try library.commit()

        let selection = try #require(
            TexturePackLibrary.sharedSelection(directory: fixture.directory)
        )
        #expect(selection.packs.map(\.id) == [second.id, first.id])
        #expect(selection.packs.map(\.name) == [second.name, first.name])
        #expect(selection.missing.isEmpty)
        #expect(!selection.unreadable)

        try FileManager.default.removeItem(at: library.fileURL(for: first.id))
        let missing = try #require(
            TexturePackLibrary.sharedSelection(directory: fixture.directory)
        )
        #expect(missing.packs.map(\.id) == [second.id])
        #expect(missing.missing.map(\.id) == [first.id])
        #expect(missing.missing.first?.name == first.name)

        library.setEnabledIDs([])
        try library.commit()
        #expect(TexturePackLibrary.sharedSelection(directory: fixture.directory) == nil)
    }

    @Test("Reports an unreadable Quick Look handoff")
    func reportsUnreadableSelection() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        try Data("invalid".utf8).write(
            to: fixture.directory.appendingPathComponent("Selection.json")
        )

        let selection = try #require(
            TexturePackLibrary.sharedSelection(directory: fixture.directory)
        )
        #expect(selection.unreadable)
        #expect(selection.packs.isEmpty)
        #expect(selection.missing.isEmpty)
    }

    @Test("Reads the legacy single-pack Quick Look handoff")
    func readsLegacySelectionFile() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let id = UUID()
        let archive = fixture.directory
            .appendingPathComponent(id.uuidString)
            .appendingPathExtension("zip")
        try Data([1, 2, 3]).write(to: archive)
        let legacy = #"{"selectedID":"\#(id.uuidString)"}"#
        try Data(legacy.utf8).write(
            to: fixture.directory.appendingPathComponent("Selection.json")
        )

        let selection = try #require(
            TexturePackLibrary.sharedSelection(directory: fixture.directory)
        )
        #expect(selection.packs == [
            TexturePackLibrary.SharedPack(
                id: id, name: "Selected texture pack", url: archive
            ),
        ])
    }

    @Test("Migrates the old single selection to the enabled stack")
    func migratesLegacySelection() throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let id = UUID()
        let archive = fixture.directory
            .appendingPathComponent(id.uuidString)
            .appendingPathExtension("zip")
        try Data([1, 2, 3]).write(to: archive)
        let legacy = """
        {"packs":[{"id":"\(id.uuidString)","name":"Legacy"}],"selectedID":"\(id.uuidString)"}
        """
        fixture.defaults.set(Data(legacy.utf8), forKey: "texturePackLibrary")

        var library = try TexturePackLibrary(
            defaults: fixture.defaults, directory: fixture.directory,
            publishesSelection: true, selectionDirectory: fixture.directory
        )
        #expect(library.enabledIDs == [id])
        #expect(library.needsMigration)

        try library.persistMigrationIfNeeded()
        #expect(!library.needsMigration)

        let stored = try #require(fixture.defaults.data(forKey: "texturePackLibrary"))
        let object = try JSONSerialization.jsonObject(with: stored) as? [String: Any]
        let json = try #require(object)
        #expect(json["enabledIDs"] != nil)
        #expect(json["selectedID"] == nil)

        let selection = try #require(
            TexturePackLibrary.sharedSelection(directory: fixture.directory)
        )
        #expect(selection.packs.map(\.id) == [id])
        #expect(selection.packs.first?.name == "Legacy")
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
        legacy.setEnabledIDs([pack.id])
        try legacy.commit()
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
        #expect(migrated.enabledIDs == [pack.id])
        #expect(try Data(contentsOf: migrated.fileURL(for: pack.id)) == legacyArchive)
        #expect(!FileManager.default.fileExists(atPath: legacyDirectory.path))
        #expect(legacyDefaults.data(forKey: "texturePackLibrary") == nil)

        let selection = try #require(
            TexturePackLibrary.sharedSelection(directory: destinationDirectory)
        )
        #expect(selection.packs.map(\.id) == [pack.id])

        // A second migration run does nothing. The shared destination is
        // already the source of truth.
        migrated.setEnabledIDs([])
        try migrated.commit()
        #expect(TexturePackLibrary.migrateLegacyState(
            from: legacyDefaults, legacyDirectory: legacyDirectory,
            to: fixture.defaults, directory: destinationDirectory
        ))
        #expect(try fixture.open().enabledIDs.isEmpty)
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
        legacy.setEnabledIDs([pack.id])
        try legacy.commit()

        #expect(TexturePackLibrary.migrateLegacyState(
            from: legacyDefaults, legacyDirectory: legacyDirectory,
            to: fixture.defaults, directory: destinationDirectory
        ))

        #expect(try fixture.open().packs == [TexturePackLibrary.Pack(
            id: shared.packs[0].id, name: "Shared"
        )])
        #expect(try fixture.open().enabledIDs.isEmpty)
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
