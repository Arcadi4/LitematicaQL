import Foundation

/// Whether the preview renders a schematic past the preview quotas without
/// the render-anyway alert. The app's settings set this flag.
///
/// When enabled, both native stages take their override entry points and
/// read past the preview quotas. The flag is a JSON file in the app-group
/// container, not a defaults key. See `TexturePackLibrary` for the reason.
@MainActor
enum RenderLimits {
    private struct SavedFlag: Codable {
        var bypassesPreviewLimits = false
    }

    private static let flagFile = "RenderLimits.json"

    /// Whether oversized schematics render without the render-anyway alert.
    /// A missing or unreadable file means false, so the preview quotas apply.
    static func bypassesPreviewLimits(directory: URL? = nil) -> Bool {
        for url in candidateURLs(directory) {
            if let data = try? Data(contentsOf: url),
               let flag = try? JSONDecoder().decode(SavedFlag.self, from: data) {
                return flag.bypassesPreviewLimits
            }
        }
        return false
    }

    /// Writes the flag to the first candidate URL that accepts the write.
    /// Atomic and best-effort: a failed write leaves the stored value
    /// unchanged. `directory` overrides the candidate URLs, for tests.
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

    /// The app-group container first, then this host's Application Support, so
    /// the app and the Quick Look extension read the same file. Ad-hoc
    /// development builds resolve the group path, but the group write can
    /// fail. The fallback gives them a writable location.
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
