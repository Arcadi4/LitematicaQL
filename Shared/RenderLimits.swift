import Foundation

/// Whether the preview's over-limit behavior is "render anyway, without
/// asking", set in the app's settings.
///
/// When enabled, both native stages take their override entry points, which
/// lift the preview quotas and levy nothing more, and the render-anyway alert
/// never appears. The flag lives in the app-group container, because the
/// Quick Look extension is a separate process and `UserDefaults(suiteName:)`
/// does not resolve group suites inside every extension runtime even when the
/// group container itself is accessible (see `TexturePackLibrary`'s selection
/// handoff, which this file mirrors).
@MainActor
enum RenderLimits {
    private struct SavedFlag: Codable {
        var bypassesPreviewLimits = false
    }

    private static let flagFile = "RenderLimits.json"

    /// Whether oversized schematics render without the render-anyway alert.
    /// A missing or unreadable file means no: the quotas stay enforced.
    static func bypassesPreviewLimits(directory: URL? = nil) -> Bool {
        for url in candidateURLs(directory) {
            if let data = try? Data(contentsOf: url),
               let flag = try? JSONDecoder().decode(SavedFlag.self, from: data) {
                return flag.bypassesPreviewLimits
            }
        }
        return false
    }

    /// Records the choice where both hosts read it, trying the app-group
    /// container first and falling back to the host's own Application Support
    /// when the container refuses access — ad-hoc development builds can
    /// resolve the group path yet fail the write. Atomic and best-effort: a
    /// failed write leaves the previous behavior in place. `directory`
    /// overrides the container chain, for tests.
    static func setBypassesPreviewLimits(_ enabled: Bool, directory: URL? = nil) {
        guard let data = try? JSONEncoder().encode(SavedFlag(bypassesPreviewLimits: enabled)) else { return }
        for url in candidateURLs(directory) {
            do {
                try FileManager.default.createDirectory(
                    at: url.deletingLastPathComponent(), withIntermediateDirectories: true
                )
                try data.write(to: url, options: .atomic)
                return
            } catch {
                continue
            }
        }
    }

    /// The shared container first, so app and Quick Look read the same file;
    /// then this host's Application Support, mirroring the texture-pack
    /// library's fallback for ad-hoc builds.
    private static func candidateURLs(_ directory: URL?) -> [URL] {
        if let directory { return [directory.appendingPathComponent(flagFile)] }
        var urls: [URL] = []
        if let group = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: TexturePackLibrary.groupIdentifier
        ) {
            urls.append(group.appendingPathComponent(flagFile))
        }
        if let support = FileManager.default.urls(
            for: .applicationSupportDirectory, in: .userDomainMask
        ).first {
            urls.append(
                support.appendingPathComponent("LitematicaQL", isDirectory: true)
                    .appendingPathComponent(flagFile)
            )
        }
        return urls
    }
}
