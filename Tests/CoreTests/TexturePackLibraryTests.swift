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

    @Test("Migrates the original saved pack without moving its archive", arguments: [true, false])
    func migratesSinglePack(wasActive: Bool) throws {
        let fixture = try LibraryFixture()
        defer { fixture.cleanUp() }
        let pack = TexturePackLibrary.Pack(id: UUID(), name: "Existing Pack")
        let url = fixture.directory.appendingPathComponent(pack.id.uuidString).appendingPathExtension("zip")
        try Data([4, 5, 6]).write(to: url)
        fixture.defaults.set(try JSONEncoder().encode(pack), forKey: "texturePackSelection")
        fixture.defaults.set(wasActive ? "custom" : "vanilla", forKey: "textureRenderingMode")

        let library = try fixture.open()

        #expect(library.packs == [pack])
        #expect(library.selectedID == (wasActive ? pack.id : nil))
        #expect(try Data(contentsOf: library.fileURL(for: pack.id)) == Data([4, 5, 6]))
        #expect(try fixture.open().packs == [pack])
        #expect(fixture.defaults.data(forKey: "texturePackSelection") == nil)
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
