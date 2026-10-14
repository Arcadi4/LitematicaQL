import Foundation
import Testing
@testable import LitematicaQLCore

@Suite("Render limit bypass")
@MainActor
struct RenderLimitsTests {
    @Test("Enforces the quotas when nothing is stored")
    func defaultsToEnforced() throws {
        let fixture = try FlagFixture()
        defer { fixture.cleanUp() }

        #expect(!RenderLimits.bypassesPreviewLimits(directory: fixture.directory))
    }

    @Test("Persists the bypass choice for the next preview in either host")
    func persistsChoice() throws {
        let fixture = try FlagFixture()
        defer { fixture.cleanUp() }

        RenderLimits.setBypassesPreviewLimits(true, directory: fixture.directory)
        #expect(RenderLimits.bypassesPreviewLimits(directory: fixture.directory))

        RenderLimits.setBypassesPreviewLimits(false, directory: fixture.directory)
        #expect(!RenderLimits.bypassesPreviewLimits(directory: fixture.directory))
    }

    @Test("Falls back to enforcing the quotas when the stored flag is unreadable")
    func unreadableFlagDefaultsToEnforced() throws {
        let fixture = try FlagFixture()
        defer { fixture.cleanUp() }

        try Data("not json".utf8).write(
            to: fixture.directory.appendingPathComponent("RenderLimits.json")
        )
        #expect(!RenderLimits.bypassesPreviewLimits(directory: fixture.directory))
    }
}

@MainActor
private struct FlagFixture {
    let directory = FileManager.default.temporaryDirectory
        .appendingPathComponent(UUID().uuidString, isDirectory: true)

    init() throws {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
    }

    func cleanUp() {
        try? FileManager.default.removeItem(at: directory)
    }
}
