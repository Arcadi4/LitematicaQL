import SwiftUI
import UniformTypeIdentifiers

struct ContentView: View {
    @State private var errorMessage: String?
    @State private var isImporterPresented = false
    @State private var selectedURL: URL?

    var body: some View {
        ZStack(alignment: .bottom) {
            if let selectedURL {
                preview(for: selectedURL)
            } else {
                welcome
            }

            if let errorMessage {
                Label(errorMessage, systemImage: "exclamationmark.triangle.fill")
                    .font(.callout)
                    .foregroundStyle(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(.red.opacity(0.88), in: Capsule())
                    .padding(20)
            }
        }
        .frame(minWidth: 680, minHeight: 480)
        .background(Color(nsColor: .windowBackgroundColor))
        .fileImporter(
            isPresented: $isImporterPresented,
            allowedContentTypes: [.litematic],
            allowsMultipleSelection: false,
            onCompletion: handleImport
        )
        .dropDestination(for: URL.self, action: handleDrop)
        .onOpenURL(perform: open)
        .toolbar {
            ToolbarItemGroup(placement: .primaryAction) {
                if selectedURL != nil {
                    Button("Home", systemImage: "house") {
                        selectedURL = nil
                        errorMessage = nil
                    }
                    .help("Return to the LitematicaQL home screen")
                }

                Button("Open", systemImage: "folder") {
                    isImporterPresented = true
                }
                .keyboardShortcut("o")
                .help("Open a Litematica schematic")
            }
        }
    }

    private var welcome: some View {
        VStack(spacing: 28) {
            BlueprintMark()
                .frame(width: 112, height: 112)

            VStack(spacing: 9) {
                Text("LitematicaQL")
                    .font(.system(size: 34, weight: .bold, design: .rounded))
                Text("Interactive Quick Look previews for Litematica schematics")
                    .font(.title3)
                    .foregroundStyle(.secondary)
            }

            HStack(spacing: 12) {
                Button("Open Schematic…") {
                    isImporterPresented = true
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.large)

                if demoURL != nil {
                    Button("View Example") {
                        if let demoURL {
                            open(demoURL)
                        }
                    }
                    .buttonStyle(.bordered)
                    .controlSize(.large)
                }
            }

            VStack(alignment: .leading, spacing: 12) {
                Label("Move LitematicaQL to Applications and open it once.", systemImage: "app.badge")
                Label("Select any .litematic file in Finder and press Space.", systemImage: "space")
                Label("Drag to orbit, scroll to zoom, and right-drag to pan.", systemImage: "rotate.3d")
            }
            .font(.callout)
            .foregroundStyle(.secondary)
            .padding(18)
            .background(.quaternary.opacity(0.45), in: RoundedRectangle(cornerRadius: 16))

            Text("Schematics stay on your Mac. The previewer never uses the network.")
                .font(.caption)
                .foregroundStyle(.tertiary)
        }
        .padding(48)
    }

    private func preview(for url: URL) -> some View {
        SchematicWebView(fileURL: url, errorMessage: $errorMessage)
            .id(url)
        .navigationTitle(url.lastPathComponent)
    }

    private var demoURL: URL? {
        Bundle.main.url(forResource: "LitematicaQL-Demo", withExtension: "litematic")
    }

    private func handleImport(_ result: Result<[URL], Error>) {
        switch result {
        case let .success(urls):
            guard let url = urls.first else {
                return
            }
            open(url)
        case let .failure(error):
            errorMessage = error.localizedDescription
        }
    }

    private func handleDrop(_ urls: [URL], _: CGPoint) -> Bool {
        guard let url = urls.first(where: {
            $0.pathExtension.lowercased() == LitematicFile.fileExtension
        }) else {
            errorMessage = LitematicFileError.unsupportedExtension.localizedDescription
            return false
        }

        open(url)
        return true
    }

    private func open(_ url: URL) {
        guard url.pathExtension.lowercased() == LitematicFile.fileExtension else {
            errorMessage = LitematicFileError.unsupportedExtension.localizedDescription
            return
        }

        errorMessage = nil
        selectedURL = url
    }
}

private struct BlueprintMark: View {
    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: 28, style: .continuous)
                .fill(
                    LinearGradient(
                        colors: [Color(red: 0.08, green: 0.17, blue: 0.20),
                                 Color(red: 0.10, green: 0.29, blue: 0.23)],
                        startPoint: .topLeading,
                        endPoint: .bottomTrailing
                    )
                )
                .shadow(color: .black.opacity(0.22), radius: 22, y: 10)

            Image(systemName: "cube.transparent")
                .font(.system(size: 54, weight: .medium))
                .foregroundStyle(Color(red: 0.55, green: 0.90, blue: 0.70))
        }
        .accessibilityHidden(true)
    }
}
